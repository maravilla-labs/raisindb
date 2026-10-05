//! Storage initialization and setup.
//!
//! This module handles the initialization of storage backends
//! including RocksDB configuration and replication state restoration.

use std::sync::Arc;

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;

use super::MergedConfig;

/// Initialize the storage backend based on configuration.
#[cfg(feature = "storage-rocksdb")]
pub fn init_storage(server_config: &MergedConfig) -> Arc<RocksDBStorage> {
    use raisin_rocksdb::RocksDBConfig;

    let mut config = RocksDBConfig::production()
        .with_path(&server_config.data_dir)
        .with_trigger_safety(raisin_rocksdb::TriggerSafetyConfig {
            enabled: server_config.trigger_safety.enabled,
            rate_limit_per_window: server_config.trigger_safety.rate_limit_per_window,
            rate_limit_hard_ceiling: server_config.trigger_safety.rate_limit_hard_ceiling,
            node_fire_budget: server_config.trigger_safety.node_fire_budget,
            window_secs: server_config.trigger_safety.window_secs,
        });

    if let Some(max_active) = server_config.max_active_jobs_per_tenant {
        config = config.with_max_active_jobs_per_tenant(Some(max_active));
    }

    if let Some(size) = server_config.storage.block_cache_size {
        config.block_cache_size = size;
    }
    if let Some(size) = server_config.storage.db_write_buffer_size {
        config.db_write_buffer_size = size;
    }
    apply_maintenance_settings(&mut config, &server_config.storage, server_config.dev_mode);
    if let Some(skip) = server_config.storage.index_skip_unchanged {
        config.index_skip_unchanged = skip;
    }
    if let Some(collapse) = server_config.storage.history_gc_collapse_runs {
        config.history_gc_collapse_runs = collapse;
    }

    tracing::info!(
        block_cache_mb = config.block_cache_size / (1024 * 1024),
        db_write_buffer_mb = config.db_write_buffer_size / (1024 * 1024),
        write_buffer_mb = config.write_buffer_size / (1024 * 1024),
        max_write_buffer_number = config.max_write_buffer_number,
        "RocksDB memory bounds"
    );

    // The production preset turns operation capture ON, and this used to only
    // ever turn it on further — never off. So every single-node server wrote
    // every operation into `operation_log` with no peer to ship it to and
    // nothing to trim it (a local dev database carried ~450 MB of it), and
    // logged two warnings per write while doing so. Capture is a replication
    // feature: off unless replication is actually configured below.
    config.replication_enabled = false;
    // The replication coordinator starts from a node id and a replication
    // port alone (`startup/replication.rs`), whatever `replication.enabled`
    // says, and then applies peers' operations at their original revisions.
    // Whatever the storage layer must refuse on a replicating node
    // (run-collapse GC) keys off this, not off operation capture.
    config.replication_configured = replication_configured(server_config);
    if server_config.replication_enabled {
        if let Some(ref node_id) = server_config.cluster_node_id {
            config.cluster_node_id = Some(node_id.clone());
            config.replication_enabled = true;
            tracing::info!("Replication enabled for node: {}", node_id);
            // Peers' HTTP addresses, for admin operations that must reach
            // every node (index repairs fan out over them).
            config.replication_peers = server_config
                .replication_peers
                .iter()
                .filter_map(|p| {
                    let url = p.http_url.as_ref()?;
                    Some(raisin_rocksdb::ReplicationPeerConfig::new(&p.peer_id, url))
                })
                .collect();
        } else {
            tracing::warn!(
                "Replication enabled but no cluster_node_id provided - replication will be disabled"
            );
        }
    }

    Arc::new(RocksDBStorage::with_config(config).expect("open rocksdb"))
}

/// Whether any cluster or replication setting is present: enough for a
/// replication coordinator (or its checkpoint ingestor) to run.
#[cfg(feature = "storage-rocksdb")]
fn replication_configured(server_config: &MergedConfig) -> bool {
    server_config.cluster_node_id.is_some()
        || server_config.replication_port.is_some()
        || !server_config.replication_peers.is_empty()
        || server_config.replication_enabled
}

/// A numeric override from the environment: `Some(Some(n))` for a number,
/// `Some(None)` for `none`/`off`, `None` when unset or unparsable.
#[cfg(feature = "storage-rocksdb")]
fn env_limit<T: std::str::FromStr>(name: &str) -> Option<Option<T>> {
    let raw = std::env::var(name).ok()?;
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("none") || raw.eq_ignore_ascii_case("off") {
        return Some(None);
    }
    match raw.parse() {
        Ok(v) => Some(Some(v)),
        Err(_) => {
            tracing::warn!(var = name, value = raw, "ignoring unparsable setting");
            None
        }
    }
}

/// History retention, job retention and the maintenance schedule: environment
/// over TOML `[storage]` over the defaults — 30 days (1 day with `--dev-mode`,
/// where a local database churns through deploys and data ticks and nobody
/// restores last month's page) or at least the last 100 revisions, 24 h of
/// job history, a pass every 6 hours.
#[cfg(feature = "storage-rocksdb")]
fn apply_maintenance_settings(
    config: &mut raisin_rocksdb::RocksDBConfig,
    storage: &crate::config::StorageConfig,
    dev_mode: bool,
) {
    use raisin_rocksdb::management::history_gc::HistoryRetention;

    let default_days = if dev_mode { 1 } else { 30 };
    let keep_days = env_limit::<u32>("RAISIN_HISTORY_KEEP_DAYS")
        .unwrap_or(Some(storage.history_keep_days.unwrap_or(default_days)));
    let keep_revisions = env_limit::<u64>("RAISIN_HISTORY_KEEP_REVISIONS")
        .unwrap_or(Some(storage.history_keep_revisions.unwrap_or(100)));
    config.history_retention = HistoryRetention {
        keep_days,
        keep_revisions,
    };
    config.job_retention_hours = env_limit::<i64>("RAISIN_JOB_RETENTION_HOURS")
        .flatten()
        .or(storage.job_retention_hours)
        .unwrap_or(24)
        .max(1);
    let minutes = env_limit::<u64>("RAISIN_MAINTENANCE_INTERVAL_MINUTES")
        .map(|v| v.unwrap_or(0))
        .or(storage.maintenance_interval_minutes)
        .unwrap_or(360);
    config.maintenance_interval_secs = minutes * 60;

    tracing::info!(
        history_keep_days = ?config.history_retention.keep_days,
        history_keep_revisions = ?config.history_retention.keep_revisions,
        job_retention_hours = config.job_retention_hours,
        maintenance_interval_minutes = minutes,
        "Storage retention"
    );
}

/// Restore replication state if enabled.
#[cfg(feature = "storage-rocksdb")]
pub async fn restore_replication_state(storage: &Arc<RocksDBStorage>) {
    if storage.config().replication_enabled {
        tracing::info!("Restoring replication vector clocks from operation log...");
        if let Err(e) = storage.restore_all_replication_state().await {
            tracing::error!(
                error = %e,
                "Failed to restore replication state; replication will be inconsistent until resolved"
            );
        }
    }
}

/// Run format and schema migrations.
#[cfg(feature = "storage-rocksdb")]
pub async fn run_migrations(storage: &Arc<RocksDBStorage>) {
    use crate::migrations;

    tracing::info!("Checking for format migration...");
    storage
        .run_format_migration()
        .await
        .expect("format migration failed");

    tracing::info!("Running schema migrations...");
    migrations::run_migrations(storage.db().clone())
        .await
        .expect("schema migration failed");
}

/// Initialize authentication service.
#[cfg(feature = "storage-rocksdb")]
pub fn init_auth_service(
    storage: &Arc<RocksDBStorage>,
    dev_mode: bool,
) -> Arc<raisin_rocksdb::AuthService> {
    use raisin_rocksdb::{AdminUserStore, AuthService};

    tracing::info!("Initializing authentication service...");

    // Capture-backed, so admin users replicate. `AdminUserStore` has always
    // called `capture_user_operation` on create/update/delete, but this call
    // site constructed it without a capture handle, which made every one of
    // those calls a silent no-op — admin users existed only on the node that
    // created them.
    let admin_user_store =
        AdminUserStore::new_with_capture(storage.db().clone(), storage.operation_capture().clone());
    // One reader for the rule (unset, empty and the historical placeholder are
    // all "not configured"); see `raisin_crypto::env_secrets`.
    let jwt_secret = match raisin_crypto::jwt_secret() {
        Some(secret) => secret,
        None if dev_mode => {
            tracing::warn!("Using default JWT secret (dev-mode)");
            raisin_crypto::INSECURE_JWT_DEFAULT.to_string()
        }
        None => {
            // The preflight in main.rs already exits before reaching here,
            // but keep this as a defense-in-depth check.
            tracing::error!("JWT_SECRET not set — refusing to start without --dev-mode");
            std::process::exit(1);
        }
    };

    // Likewise for API keys: one that exists only where it was minted fails
    // authentication everywhere else in the cluster.
    Arc::new(AuthService::new_with_capture(
        admin_user_store,
        jwt_secret,
        storage.operation_capture().clone(),
    ))
}
