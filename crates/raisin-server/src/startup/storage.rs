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

    let mut config = RocksDBConfig::production().with_path(&server_config.data_dir);

    if server_config.replication_enabled {
        if let Some(ref node_id) = server_config.cluster_node_id {
            config.cluster_node_id = Some(node_id.clone());
            config.replication_enabled = true;
            tracing::info!("Replication enabled for node: {}", node_id);
        } else {
            tracing::warn!(
                "Replication enabled but no cluster_node_id provided - replication will be disabled"
            );
        }
    }

    Arc::new(RocksDBStorage::with_config(config).expect("open rocksdb"))
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

    let admin_user_store = AdminUserStore::new(storage.db().clone());
    let jwt_secret = std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| "default_jwt_secret_change_in_production".to_string());

    if jwt_secret == "default_jwt_secret_change_in_production" {
        if dev_mode {
            tracing::warn!("Using default JWT secret (dev-mode)");
        } else {
            // Startup validation in main.rs already exits before reaching here,
            // but keep this as a defense-in-depth check.
            tracing::error!("JWT_SECRET not set — refusing to start without --dev-mode");
            std::process::exit(1);
        }
    }

    Arc::new(AuthService::new(admin_user_store, jwt_secret))
}
