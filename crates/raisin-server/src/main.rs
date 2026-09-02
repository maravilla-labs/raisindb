//  TODO(v0.2): Clean up unused code and deprecated usages
#![allow(deprecated)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unexpected_cfgs)]

/// jemalloc global allocator (Unix). glibc malloc never returns arena memory
/// to the OS under the function-runtime's churny multi-threaded allocation
/// pattern, growing RSS unboundedly; jemalloc purges freed pages on decay.
#[cfg(unix)]
#[global_allocator]
static GLOBAL_ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod admin_ui;
mod admin_user_init_handler;
#[cfg(feature = "storage-rocksdb")]
mod builtin_package_init_handler;
mod config;
mod deps_setup;
mod diagnostics;
mod encrypted_fields_event_handler;
#[cfg(feature = "storage-rocksdb")]
mod flow_sweeper;
mod function_module_event_handler;
mod management;
#[cfg(feature = "storage-rocksdb")]
mod migrations;
mod nodetype_init_handler;
mod schema_stats_event_handler;
mod sse;
mod startup;
mod workspace_catalog_event_handler;
mod workspace_init_handler;
mod workspace_structure_init_handler;

use axum::{
    routing::{any, get},
    Router,
};
use clap::Parser;
#[cfg(not(feature = "storage-rocksdb"))]
use raisin_audit::InMemoryAuditRepo;
use raisin_binary::BinaryStorage;
use raisin_core::{RepoAuditAdapter, WorkspaceService};
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{RegistryRepository, Storage};
#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;
use raisin_transport_http as http;
#[cfg(feature = "websocket")]
use raisin_transport_ws::{websocket_handler, websocket_handler_tenantless, WsConfig, WsState};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    // Parse command-line arguments and merge configuration sources
    let cli_config = startup::ServerConfig::parse();
    let mut server_config = cli_config.merge().expect("Failed to load configuration");

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("RaisinDB Server starting...");
    tracing::info!("  HTTP Port: {}", server_config.port);
    tracing::info!("  Data Directory: {}", server_config.data_dir);

    // Load C-ABI function-binding plugins (`.so`/`.dylib`/`.dll`) from the plugin
    // directory — `RAISIN_PLUGIN_DIR`, else `<data_dir>/plugins`. A distribution
    // drops a per-platform plugin lib here to add `raisin.<ns>.*` bindings to the
    // stock binary with no custom build. Must run before any function executes.
    #[cfg(feature = "storage-rocksdb")]
    {
        let plugin_dir = std::env::var("RAISIN_PLUGIN_DIR")
            .unwrap_or_else(|_| format!("{}/plugins", server_config.data_dir));
        tracing::info!(plugin_dir = %plugin_dir, "scanning for function plugins");
        raisin_functions::load_plugins_from_dir(&plugin_dir);

        // Tell the layers BELOW the plugin registry what this process can do.
        //
        // `raisin-ai` owns the task planner and `raisin-rocksdb` owns the asset
        // job, and both sit under `raisin-functions` — Cargo rejects the cycle,
        // so neither can ask `plugin_method_available` directly. The enqueue
        // path therefore planned against a hard-coded "no plugins", which made
        // every delegated task report `plugin_missing` on the one kind of
        // server that could actually perform it. Installing the probe here is
        // the same inversion as `configure_mcp_client` and
        // `configure_platform_hooks` below: one policy, installed once,
        // process-wide.
        //
        // Deliberately installed even when no plugin loaded. The probe then
        // answers false for everything, which is what the hard-coded predicate
        // did — but `capability_probe_installed()` can now distinguish "nothing
        // can run here" from "nobody has said yet".
        if !raisin_ai::install_capability_probe(std::sync::Arc::new(
            raisin_functions::plugin::plugin_method_available,
        )) {
            tracing::warn!(
                "capability probe was already installed; keeping the first one. \
                 Two answers to \"what can this server do\" is the drift the \
                 registry exists to prevent"
            );
        }

        // Say on BOOT what this server can actually do with a binary asset —
        // "docx: plugin OK (maravilla-media)", "pdf: core", "mp4: UNSUPPORTED" —
        // and name every plugin file the loader refused and why. A rejected
        // plugin (an ABI bump above all) leaves a server that boots perfectly
        // and has silently lost `raisin.media.*` for every tenant; the only
        // previous signal was one ERROR line in the journal, which no deploy
        // asserts on. See CORE-PLUGIN-DEPLOYMENT.md §5.
        raisin_functions::log_capability_report();
    }

    // ========================================================================
    // Dev-mode banner & production secret validation
    // ========================================================================

    // Record it process-wide BEFORE anything can sign. `signing_secret_or_dev`
    // is the one place the dev fallback is applied, and it reads this flag —
    // the HTTP transport and the `raisin.assets.signedUrl` binding both mint
    // asset URLs, and two different answers here would be two different keys.
    raisin_crypto::set_dev_mode(server_config.dev_mode);

    if server_config.dev_mode {
        tracing::warn!("============================================================");
        tracing::warn!("  DEV-MODE ENABLED — insecure defaults are allowed.");
        tracing::warn!("  Do NOT use --dev-mode in production!");
        tracing::warn!("============================================================");

        // Default the locks/inventory subsystem on (in-process) for dev-mode so
        // `raisin.inventory.claim/release` work out of the box without a config
        // file. A single-node in-process backend is correct for local dev. An
        // explicitly-enabled `[locks]` section (e.g. backend = redis) is left
        // untouched; we only flip it on when it would otherwise be disabled.
        if !server_config.locks.enabled {
            server_config.locks.enabled = true;
            server_config.locks.backend = raisin_locks::LockBackend::InProcess;
            tracing::warn!("  Locks subsystem defaulted to in-process (dev-mode).");
        }
    } else {
        // In production mode every required secret must be set. This used to
        // be three `if missing { error; exit(1) }` blocks, so an operator
        // bringing up a node learned the requirements ONE BOOT AT A TIME —
        // and the master-key block checked variable NAMES, which meant a
        // deployment that had migrated to the `RAISIN_MASTER_KEYS` keyring
        // could not start at all even though every runtime reader accepts it.
        // `raisin_crypto::env_secrets` now owns both the list and the
        // validation, checking through the same loaders the running server
        // uses, and reports every unmet requirement in one pass.
        let problems = raisin_crypto::production_secret_problems();
        if !problems.is_empty() {
            tracing::error!(
                "Refusing to start: {} required production secret(s) missing or invalid. \
                 Set all of them, or pass --dev-mode for development.",
                problems.len()
            );
            for problem in &problems {
                tracing::error!("  {}", problem);
            }
            std::process::exit(1);
        }
    }

    // Install the secret-vaulting policy before ANY write path can run — this
    // has to be ahead of storage init, the job system and both transports,
    // because the first node write is the first chance to leak a plaintext
    // secret. Process-global for the same reason as the MCP egress policy
    // below: the write paths that consult it sit deep enough that threading a
    // handle to each would mean several near-identical parameters, and one of
    // them disagreeing would be silent.
    #[cfg(feature = "storage-rocksdb")]
    raisin_rocksdb::vaulting::configure_vaulting(server_config.secrets.vaulting_enabled);

    // Run external dependency setup (Tesseract OCR, etc.)
    tracing::info!("Checking external dependencies...");
    if let Err(e) = deps_setup::run_dependency_setup(&server_config.data_dir) {
        tracing::error!(error = %e, "Failed to run dependency setup");
    }
    if server_config.anonymous_enabled {
        tracing::info!("  Anonymous Access: ENABLED");
    }
    if let Some(ref node_id) = server_config.cluster_node_id {
        tracing::info!("  Cluster Node ID: {}", node_id);
    }
    if let Some(ref repl_port) = server_config.replication_port {
        tracing::info!("  Replication Port: {}", repl_port);
    }

    // ========================================================================
    // Storage initialization
    // ========================================================================

    #[cfg(feature = "storage-rocksdb")]
    let storage = startup::storage::init_storage(&server_config);

    #[cfg(not(any(feature = "storage-rocksdb", feature = "storage-rocksdb")))]
    let storage = Arc::new(InMemoryStorage::default());

    #[cfg(feature = "storage-rocksdb")]
    startup::storage::restore_replication_state(&storage).await;

    // Extract values needed after storage is moved by init_job_system()
    #[cfg(feature = "storage-rocksdb")]
    let index_path = storage.config().path.join("tantivy_indexes");
    #[cfg(feature = "storage-rocksdb")]
    let hnsw_path = storage.config().path.join("hnsw_indexes");
    #[cfg(feature = "storage-rocksdb")]
    let db_clone = storage.db().clone();

    #[cfg(feature = "storage-rocksdb")]
    startup::storage::run_migrations(&storage).await;

    // ========================================================================
    // Authentication
    // ========================================================================

    #[cfg(feature = "storage-rocksdb")]
    let auth_service = startup::storage::init_auth_service(&storage, server_config.dev_mode);

    // ========================================================================
    // Event handlers
    // ========================================================================

    startup::events::register_event_handlers(storage.clone());

    // Create schema stats cache and register invalidation event handler
    let schema_stats_cache = raisin_core::new_shared_schema_stats_cache_default();
    {
        let event_bus = storage.event_bus();
        event_bus.subscribe(std::sync::Arc::new(
            schema_stats_event_handler::SchemaStatsEventHandler::new(schema_stats_cache.clone()),
        ));

        // The encrypted-fields gate rides the same event, for a much sharper
        // reason: a stale statistic costs a worse plan, a stale `false` there
        // writes a secret to disk in plaintext. Its checkpoint-ingest
        // invalidation is already registered inside `encrypted_fields::global()`.
        event_bus.subscribe(std::sync::Arc::new(
            encrypted_fields_event_handler::EncryptedFieldsEventHandler,
        ));

        // Replication checkpoint ingestion copies the node_types / archetypes
        // column families straight into the live database and emits NO events by
        // design, so the per-scope invalidation above never fires for it. The
        // workspace catalog already registers here for exactly this reason
        // (`engine::catalog_cache`); the schema stats cache did not, so a replica
        // that caught up by checkpoint kept planning against the counts it held
        // before the ingest until the 5-minute TTL floor expired.
        //
        // Stale counts are a PLANNING fault, not a wrong-answer fault — the
        // planner picks a worse compound index — which is precisely why it would
        // never have surfaced as a bug report.
        let stats_for_ingest = schema_stats_cache.clone();
        raisin_core::register_invalidator(move || stats_for_ingest.invalidate_all());

        tracing::info!("Schema stats cache initialized with event-driven invalidation");
    }

    // The SQL catalog is derived once per repo and shared by every transport;
    // this handler is what keeps a newly created workspace immediately queryable.
    {
        let event_bus = storage.event_bus();
        event_bus.subscribe(std::sync::Arc::new(
            workspace_catalog_event_handler::WorkspaceCatalogEventHandler,
        ));
        tracing::info!("Workspace catalog cache initialized with event-driven invalidation");
    }

    // Function module cache: on by default, and this subscription is what makes
    // that safe — a function edit must never keep executing the old code.
    {
        let event_bus = storage.event_bus();
        event_bus.subscribe(std::sync::Arc::new(
            function_module_event_handler::FunctionModuleEventHandler::new("functions"),
        ));
    }

    // Register graph projection event handler for event-driven invalidation
    #[cfg(feature = "storage-rocksdb")]
    startup::events::register_graph_projection_handler(&storage);

    #[cfg(feature = "storage-rocksdb")]
    startup::events::register_admin_handler(
        &storage,
        auth_service.clone(),
        server_config.initial_admin_password.as_deref(),
    );

    #[cfg(feature = "storage-rocksdb")]
    startup::events::register_default_tenant(&storage).await;

    // ========================================================================
    // Replication coordinator (cluster mode)
    // ========================================================================

    #[cfg(feature = "storage-rocksdb")]
    let _replication_coordinator = startup::replication::start_replication_coordinator(
        &storage,
        server_config.cluster_node_id.as_deref(),
        server_config.replication_port,
        &server_config.replication_peers,
    )
    .await;

    // ========================================================================
    // Monitoring
    // ========================================================================

    #[cfg(feature = "storage-rocksdb")]
    let monitoring = {
        use raisin_rocksdb::monitoring::MonitoringService;

        tracing::info!("Initializing monitoring service...");

        let coordinator = _replication_coordinator.as_ref().cloned();
        let monitoring_service = Arc::new(MonitoringService::new(storage.clone(), coordinator));

        if server_config.monitoring_enabled {
            let interval = std::time::Duration::from_secs(server_config.monitoring_interval_secs);
            monitoring_service.start_periodic_logging(interval);
            tracing::info!(
                "Monitoring service started with {}s interval",
                interval.as_secs()
            );
        } else {
            tracing::info!("Monitoring service initialized (periodic logging disabled)");
        }

        monitoring_service
    };

    // ========================================================================
    // Indexing engines (Tantivy + HNSW)
    // ========================================================================

    #[cfg(feature = "storage-rocksdb")]
    let (indexing_engine, tantivy_management) = {
        let (engine, management) = startup::indexing::init_tantivy_engine(index_path);
        (Some(engine), Some(management))
    };

    #[cfg(feature = "storage-rocksdb")]
    let hnsw_engine = {
        let engine = startup::indexing::init_hnsw_engine(hnsw_path, &storage);
        Some(engine)
    };

    // The query-side partner of the HNSW engine above, installed process-wide so
    // that EVERY SQL surface can embed query text — not just the one that
    // remembered to.
    //
    // `with_embedding_provider` had a single call site in the whole workspace
    // (the HTTP `/api/sql` handler). pgwire simple query, pgwire extended query,
    // the WebSocket `sql_query` request and the three engine constructions
    // behind `raisin.sql()` all wired the HNSW engine and stopped there, so
    // `HYBRID_SEARCH` on those four surfaces skipped its vector half and
    // returned a plain full-text search with `vector_rank` NULL on every row and
    // nothing in the log. Installing it here rather than adding a seventh
    // `.with_embedding_provider(...)` line is deliberate: the seventh line is
    // exactly what was forgotten six times, and a surface added later inherits
    // this one automatically.
    #[cfg(feature = "storage-rocksdb")]
    {
        let embedder = std::sync::Arc::new(
            raisin_rocksdb::embedding_provider::TenantQueryEmbedderResolver::new(storage.clone()),
        );
        if raisin_embeddings::configure_query_embedder(embedder) {
            tracing::info!("Query embedder installed for all SQL surfaces");
        } else {
            tracing::warn!("Query embedder was already installed; keeping the first one");
        }
    }

    // ========================================================================
    // Binary storage
    // ========================================================================

    #[cfg(feature = "s3")]
    let bin = startup::binary::init_binary_storage().await;
    #[cfg(not(feature = "s3"))]
    let bin = startup::binary::init_binary_storage(&server_config.data_dir);

    // ========================================================================
    // System definitions (built-in NodeTypes / Workspaces / packages)
    // ========================================================================
    //
    // Embedded definitions are the baseline; an optional on-disk overlay
    // (`[system_definitions] overlay_dir`) can override any of them by name so
    // a schema fix ships without rebuilding the binary.
    // `build_and_install_resolver` also makes this the process-wide stack, so
    // the older init paths (RepositoryCreated NodeType handler, package install,
    // workspace self-heal) serve overlay definitions instead of reverting to the
    // embedded ones.
    let definitions = Arc::new(raisin_core::definitions::build_and_install_resolver(
        &server_config.system_definitions,
        &server_config.data_dir,
    ));

    #[cfg(feature = "storage-rocksdb")]
    startup::binary::register_builtin_package_handler(&storage, &bin, &definitions).await;

    // Propagate built-in NodeType/Workspace schema changes into repositories
    // created before this binary. Gated on CONTENT HASH (not the YAML `version:`
    // field), so an edit propagates whether or not anyone bumped the version;
    // breaking changes are withheld for an explicit admin apply.
    #[cfg(feature = "storage-rocksdb")]
    startup::binary::resync_system_definitions(
        &storage,
        &definitions,
        server_config.system_definitions.auto_apply,
    )
    .await;

    // ========================================================================
    // Locks / inventory subsystem
    // ========================================================================
    //
    // A single shared LockManager is constructed here and handed to every
    // surface (functions runtime, WS, HTTP) so they all coordinate against the
    // same source of truth. In-process is single-node only; the Redis backend
    // is required for correct mutual exclusion across a cluster.
    let lock_manager: Option<raisin_locks::LockManagerHandle> = match raisin_locks::build(
        &server_config.locks,
    )
    .await
    {
        Ok(Some(mgr)) => {
            tracing::info!(
                backend = ?server_config.locks.backend,
                "Locks/inventory subsystem enabled"
            );
            if server_config.locks.backend == raisin_locks::LockBackend::InProcess
                && server_config.replication_enabled
            {
                tracing::warn!(
                    "locks.backend = inprocess while replication is enabled — \
                         in-process locks do NOT coordinate across cluster nodes and \
                         will oversell. Use the Redis backend for multi-node deployments."
                );
            }
            Some(mgr)
        }
        Ok(None) => {
            tracing::info!("Locks/inventory subsystem disabled");
            None
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to initialize locks subsystem; continuing without it");
            None
        }
    };

    // ========================================================================
    // Job system initialization
    // ========================================================================

    #[cfg(feature = "storage-rocksdb")]
    let (_worker_pool, shutdown_token, _rt_runtime, _bg_runtime, _sys_runtime) = {
        if storage.config().background_jobs_enabled {
            tracing::info!("Initializing unified job system...");

            // Load the operator-set per-tenant scheduling weights into the
            // process-wide table BEFORE the pools exist, so the first job
            // dispatched is already weighted. Without this a restart would
            // flatten every tenant back to equal share — an operator's
            // configuration undone with nothing logged and nothing to see.
            // Best-effort: a read failure means equal share, which is the
            // safe direction and is what an unconfigured server does anyway.
            if let Err(e) = raisin_rocksdb::install_persisted_weights(storage.db()) {
                tracing::warn!(
                    error = %e,
                    "Could not load tenant scheduling weights; every tenant gets equal share"
                );
            }

            // Propagate EMBEDDING_MASTER_KEY → RAISIN_MASTER_KEY for backward compatibility
            if std::env::var("RAISIN_MASTER_KEY").is_err() {
                if let Ok(emk) = std::env::var("EMBEDDING_MASTER_KEY") {
                    std::env::set_var("RAISIN_MASTER_KEY", emk);
                } else if server_config.dev_mode {
                    tracing::warn!(
                        "RAISIN_MASTER_KEY not set — using insecure all-zero key (dev-mode)"
                    );
                    std::env::set_var(
                        "RAISIN_MASTER_KEY",
                        "0000000000000000000000000000000000000000000000000000000000000000",
                    );
                }
                // In production mode, startup validation already ensured the key is set
            }

            // Determine pool configuration based on environment
            let pools_config = if let Ok(preset) = std::env::var("RAISIN_JOB_POOL_PRESET") {
                match preset.as_str() {
                    "production" => raisin_rocksdb::config::JobPoolsConfig::production(),
                    "high_performance" => {
                        raisin_rocksdb::config::JobPoolsConfig::high_performance()
                    }
                    _ => raisin_rocksdb::config::JobPoolsConfig::development(),
                }
            } else if server_config.dev_mode {
                raisin_rocksdb::config::JobPoolsConfig::development()
            } else {
                raisin_rocksdb::config::JobPoolsConfig::production()
            };

            // Create three dedicated tokio runtimes for category isolation.
            // Each category gets its own thread pool so blocking operations
            // (block_in_place in QuickJS/Starlark) in one category can't
            // starve another.
            let rt_runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(pools_config.realtime.runtime_threads)
                .thread_name("raisin-realtime")
                .enable_all()
                .build()
                .expect("Failed to create realtime runtime");
            let bg_runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(pools_config.background.runtime_threads)
                .thread_name("raisin-background")
                .enable_all()
                .build()
                .expect("Failed to create background runtime");
            let sys_runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(pools_config.system.runtime_threads)
                .thread_name("raisin-system")
                .enable_all()
                .build()
                .expect("Failed to create system runtime");

            let mut worker_runtimes = std::collections::HashMap::new();
            worker_runtimes.insert(
                raisin_storage::jobs::JobCategory::Realtime,
                rt_runtime.handle().clone(),
            );
            worker_runtimes.insert(
                raisin_storage::jobs::JobCategory::Background,
                bg_runtime.handle().clone(),
            );
            worker_runtimes.insert(
                raisin_storage::jobs::JobCategory::System,
                sys_runtime.handle().clone(),
            );

            let storage_for_init = storage.clone();

            use raisin_functions::execution::{
                ExecutionDependencies, ExecutionProvider, FunctionExecutionConfig,
            };

            let execution_deps = Arc::new(ExecutionDependencies {
                storage: storage.clone(),
                binary_storage: bin.clone(),
                indexing_engine: indexing_engine.clone(),
                hnsw_engine: hnsw_engine.clone(),
                http_client: raisin_functions::shared_http_client(),
                ai_config_store: Some(Arc::new(storage.tenant_ai_config_repository())),
                job_registry: Some(storage.job_registry().clone()),
                job_data_store: Some(storage.job_data_store().clone()),
                lock_manager: lock_manager.clone(),
                // None when no master keyring is configured — the binding then
                // reports the subsystem as unconfigured rather than failing at
                // an opaque decrypt.
                secret_store: storage.secret_store().ok(),
                identity_repo: Some(Arc::new(storage.identity_repository())),
                schema_stats_cache: Some(schema_stats_cache.clone()),
                // What lets a function read a mounted asset's bytes when the
                // local cache has expired.
                //
                // A closure, because the sync engine is built by
                // `init_job_system` further down and published on the storage.
                // Reading it at CALL time is what makes that ordering
                // irrelevant; capturing the handle here would capture `None`
                // forever and mounted content would silently never load.
                mount_content: Some({
                    let storage = storage.clone();
                    Arc::new(move || storage.virtual_mount_sync_handler())
                }),
            });

            let execution_callbacks = ExecutionProvider::create_callbacks_with_deps(
                execution_deps.clone(),
                FunctionExecutionConfig::default(),
            );
            tracing::info!("Using production execution callbacks with full dependencies");

            use raisin_functions::execution::create_flow_callbacks;
            let flow_callbacks = create_flow_callbacks(execution_deps.clone());
            tracing::info!("Created flow instance execution callbacks");

            // Scheduled trigger finder: queries every `raisin:Trigger` node in
            // the `functions` workspace across all repos, filters by
            // `trigger_type = "schedule"`, evaluates the cron expression
            // against the current time, and returns matches for the handler
            // to enqueue. The periodic enqueuer below kicks a
            // ScheduledTriggerCheck job every minute.
            //
            // First cut — best-effort, untested without a running cluster.
            // Errors on individual repos are logged and the scan continues
            // so one broken tenant can't break everyone else's schedules.
            let storage_for_finder = storage.clone();
            let scheduled_trigger_finder: Option<raisin_rocksdb::ScheduledTriggerFinderCallback> = {
                use raisin_models::nodes::properties::PropertyValue;
                use raisin_storage::{
                    ListOptions, NodeRepository, RepositoryManagementRepository, Storage,
                    StorageScope,
                };
                Some(std::sync::Arc::new(
                    move |tenant_filter, repo_filter, current_time| {
                        let storage = storage_for_finder.clone();
                        Box::pin(async move {
                            let mut matches = Vec::new();
                            // Enumerate tenants -> repos via the
                            // background-job helpers: the tenant-scoped
                            // repository_management() facade returns NOTHING
                            // outside a request context (same bug as the
                            // flow sweeper had).
                            let tenants = match raisin_rocksdb::management::list_tenants(&storage)
                                .await
                            {
                                Ok(t) => t,
                                Err(e) => {
                                    tracing::warn!(error = %e, "Failed to list tenants for scheduled trigger scan");
                                    return Ok(matches);
                                }
                            };
                            let mut repos: Vec<(String, String)> = Vec::new();
                            for tenant in tenants {
                                match raisin_rocksdb::management::list_repositories(
                                    &storage, &tenant,
                                )
                                .await
                                {
                                    Ok(repo_ids) => repos
                                        .extend(repo_ids.into_iter().map(|r| (tenant.clone(), r))),
                                    Err(e) => {
                                        tracing::warn!(tenant = %tenant, error = %e, "Failed to list repositories for scheduled trigger scan");
                                    }
                                }
                            }
                            for (tenant_id, repo_id) in repos {
                                if let Some(ref t) = tenant_filter {
                                    if &tenant_id != t {
                                        continue;
                                    }
                                }
                                if let Some(ref r) = repo_filter {
                                    if &repo_id != r {
                                        continue;
                                    }
                                }
                                let branch = "main";
                                let scope =
                                    StorageScope::new(&tenant_id, &repo_id, branch, "functions");
                                let triggers = match storage
                                    .nodes()
                                    .list_by_type(scope, "raisin:Trigger", ListOptions::default())
                                    .await
                                {
                                    Ok(t) => t,
                                    Err(e) => {
                                        tracing::debug!(
                                            tenant = %tenant_id,
                                            repo = %repo_id,
                                            error = %e,
                                            "Failed to list triggers in repo (skipping)"
                                        );
                                        continue;
                                    }
                                };
                                for t in triggers {
                                    let enabled = matches!(
                                        t.properties.get("enabled"),
                                        Some(PropertyValue::Boolean(true))
                                    );
                                    if !enabled {
                                        continue;
                                    }
                                    let trigger_type = match t.properties.get("trigger_type") {
                                        Some(PropertyValue::String(s)) => s.as_str(),
                                        _ => continue,
                                    };
                                    if trigger_type != "schedule" {
                                        continue;
                                    }
                                    let cron_expr = match t.properties.get("config") {
                                        Some(PropertyValue::Object(obj)) => {
                                            match obj.get("cron_expression") {
                                                Some(PropertyValue::String(s)) => s.clone(),
                                                _ => continue,
                                            }
                                        }
                                        _ => continue,
                                    };
                                    if !raisin_rocksdb::cron_matches(&cron_expr, current_time) {
                                        continue;
                                    }
                                    let function_path = match t.properties.get("function_path") {
                                        Some(PropertyValue::String(s)) => s.clone(),
                                        _ => continue,
                                    };
                                    let trigger_name = match t.properties.get("name") {
                                        Some(PropertyValue::String(s)) => s.clone(),
                                        _ => t.id.to_string(),
                                    };
                                    matches.push(raisin_rocksdb::ScheduledTriggerMatch {
                                        function_path,
                                        trigger_name,
                                        tenant_id: tenant_id.clone(),
                                        repo_id: repo_id.clone(),
                                        branch: branch.to_string(),
                                        workspace: "functions".to_string(),
                                    });
                                }
                            }
                            Ok(matches)
                        })
                    },
                ))
            };

            let binary_storage_callback =
                startup::jobs::create_binary_storage_callback(bin.clone());
            let binary_delete_callback = startup::jobs::create_binary_delete_callback(bin.clone());

            // Binary upload callback is inlined here (not in startup/jobs.rs) because
            // put_stream's generic `S: Stream + 'a` lifetime bound requires the closure
            // to be defined in the same scope as the BinaryStorage concrete type.
            // See Rust issue #100013.
            let bin_for_upload = bin.clone();
            let binary_upload_callback: raisin_rocksdb::BinaryUploadCallback = std::sync::Arc::new(
                move |chunk_paths: Vec<std::path::PathBuf>,
                      filename: String,
                      content_type: Option<String>,
                      tenant_id: String,
                      file_size: u64| {
                    let bin = bin_for_upload.clone();
                    Box::pin(async move {
                        use futures_util::{stream, StreamExt};
                        use tokio::io::AsyncReadExt;

                        let chunk_stream = stream::iter(chunk_paths).then(|path| async move {
                            let mut file = tokio::fs::File::open(&path)
                                .await
                                .map_err(std::io::Error::other)?;
                            let mut buffer = Vec::new();
                            file.read_to_end(&mut buffer)
                                .await
                                .map_err(std::io::Error::other)?;
                            Ok::<bytes::Bytes, std::io::Error>(bytes::Bytes::from(buffer))
                        });

                        let ext = std::path::Path::new(&filename)
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|s| s.to_string());

                        bin.put_stream(
                            chunk_stream,
                            content_type.as_deref(),
                            ext.as_deref(),
                            Some(&filename),
                            Some(file_size),
                            Some(&tenant_id),
                        )
                        .await
                        .map_err(|e| raisin_error::Error::storage(e.to_string()))
                    })
                },
            );

            let ai_tool_call_node_creator =
                startup::jobs::create_node_creator_callback(storage.clone());

            let (pool, token) = storage_for_init
                .init_job_system(
                    indexing_engine.clone().unwrap(),
                    hnsw_engine.clone().unwrap(),
                    execution_callbacks.sql_executor,
                    None,
                    None,
                    execution_callbacks.function_executor,
                    execution_callbacks.function_enabled_checker,
                    scheduled_trigger_finder,
                    execution_callbacks.binary_retrieval,
                    Some(binary_storage_callback),
                    Some(binary_delete_callback),
                    Some(binary_upload_callback),
                    Some(flow_callbacks.node_loader),
                    Some(flow_callbacks.node_saver),
                    Some(flow_callbacks.node_creator),
                    Some(flow_callbacks.job_queuer),
                    Some(flow_callbacks.ai_caller),
                    flow_callbacks.ai_streaming_caller,
                    Some(flow_callbacks.function_executor),
                    Some(flow_callbacks.children_lister),
                    Some(ai_tool_call_node_creator),
                    lock_manager.clone(),
                    // Outbound MCP tool discovery. Built from [mcp_client] so
                    // the egress policy the HTTP layer enforces on save is the
                    // same one the job enforces before it dials.
                    Some(raisin_rocksdb::McpDiscoveryDeps {
                        egress: server_config.mcp_client.egress_policy(),
                        max_response_bytes: server_config.mcp_client.max_response_bytes,
                        http: raisin_functions::shared_http_client(),
                    }),
                    worker_runtimes,
                    pools_config.clone(),
                )
                .await
                .expect("Failed to initialize job system");

            let total_threads = pools_config.realtime.runtime_threads
                + pools_config.background.runtime_threads
                + pools_config.system.runtime_threads;
            let total_workers = pools_config.realtime.dispatcher_workers
                + pools_config.background.dispatcher_workers
                + pools_config.system.dispatcher_workers;
            tracing::info!("Three-pool job system started successfully");
            tracing::info!(
                "Realtime pool: {} workers, {} threads, {} max handlers",
                pools_config.realtime.dispatcher_workers,
                pools_config.realtime.runtime_threads,
                pools_config.realtime.max_concurrent_handlers,
            );
            tracing::info!(
                "Background pool: {} workers, {} threads, {} max handlers",
                pools_config.background.dispatcher_workers,
                pools_config.background.runtime_threads,
                pools_config.background.max_concurrent_handlers,
            );
            tracing::info!(
                "System pool: {} workers, {} threads, {} max handlers",
                pools_config.system.dispatcher_workers,
                pools_config.system.runtime_threads,
                pools_config.system.max_concurrent_handlers,
            );
            // Periodic enqueuer for scheduled trigger checks.
            // Every 60 seconds, register a ScheduledTriggerCheck job that the
            // handler picks up and dispatches to the finder above. Best-effort
            // — failures are logged but don't break the loop.
            //
            // TODO(cluster-coordination): in a multi-node cluster every node
            // currently runs this loop independently. Without a leader-election
            // or single-fire guarantee, a cron-fired side effect (e.g. "send
            // reminder email" function) will execute N times — once per node.
            // We need to decide on the architecture before scheduled triggers
            // are safe in clustered deployments. Options on the table:
            //   - Leader election (one node owns the enqueuer; others sit out).
            //   - Distributed lock per minute-tick keyed on the wall-clock
            //     minute, claimed via the existing job registry.
            //   - Run the enqueuer on every node but make ScheduledTriggerCheck
            //     itself a singleton job (registry-side dedup by trigger+minute).
            // Hold a separate session to pick one. Until then, only run with
            // a single raisindb node in production.
            {
                let registry_for_loop = storage.job_registry().clone();
                let job_data_store_for_loop = storage.job_data_store().clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    // Wait one tick so we don't fire immediately at boot.
                    interval.tick().await;
                    loop {
                        interval.tick().await;
                        let job_type = raisin_storage::jobs::JobType::ScheduledTriggerCheck {
                            tenant_id: None,
                            repo_id: None,
                        };
                        // The handler ignores the context, but workers refuse
                        // to dispatch jobs without one — store it BEFORE the
                        // job becomes visible to dispatch. (Registering with
                        // no context made every tick die with "Missing job
                        // context (after grace retries)".)
                        let job_id = raisin_storage::jobs::JobId::new();
                        let context = raisin_storage::jobs::JobContext {
                            tenant_id: "_system".to_string(),
                            repo_id: String::new(),
                            branch: String::new(),
                            workspace_id: String::new(),
                            revision: raisin_hlc::HLC::now(),
                            metadata: std::collections::HashMap::new(),
                        };
                        if let Err(e) = job_data_store_for_loop.put(&job_id, &context) {
                            tracing::warn!(error = %e, "Failed to store ScheduledTriggerCheck job context");
                            continue;
                        }
                        if let Err(e) = registry_for_loop
                            .register_job_with_id(
                                job_id,
                                job_type,
                                "_system".to_string(),
                                None,
                                None,
                                None,
                            )
                            .await
                        {
                            tracing::warn!(error = %e, "Failed to register ScheduledTriggerCheck job");
                        }

                        // Integration OAuth token refresh: enqueue at most once
                        // per 10-minute window. The dedup key collapses every
                        // 60s tick inside the same window to a single job, so
                        // this effectively fires every 10 minutes.
                        //
                        // Per PROCESS, not per cluster: `JobRegistry`'s dedup
                        // map is an in-memory HashMap with no storage behind
                        // it, so every node runs its own sweep. (This comment
                        // used to claim otherwise.) Cross-node safety is the
                        // per-integration lease the handler takes, which needs
                        // [locks] backend=redis to span nodes.
                        //
                        // Idempotent registration needs the context stored
                        // first, keyed by the same job id.
                        let now_secs = chrono::Utc::now().timestamp().max(0) as u64;
                        let dedup_key = raisin_rocksdb::token_refresh_dedup_key(now_secs);
                        let refresh_job = raisin_storage::jobs::JobType::IntegrationTokenRefresh {
                            tenant_id: None,
                        };
                        let refresh_job_id = raisin_storage::jobs::JobId::new();
                        let refresh_context = raisin_storage::jobs::JobContext {
                            tenant_id: "_system".to_string(),
                            repo_id: String::new(),
                            branch: String::new(),
                            workspace_id: String::new(),
                            revision: raisin_hlc::HLC::now(),
                            metadata: std::collections::HashMap::new(),
                        };
                        if let Err(e) =
                            job_data_store_for_loop.put(&refresh_job_id, &refresh_context)
                        {
                            tracing::warn!(error = %e, "Failed to store IntegrationTokenRefresh job context");
                        } else if let Err(e) = registry_for_loop
                            .register_job_with_id_idempotent(
                                refresh_job_id,
                                refresh_job,
                                "_system".to_string(),
                                dedup_key,
                                None,
                            )
                            .await
                        {
                            tracing::warn!(error = %e, "Failed to register IntegrationTokenRefresh job");
                        }

                        // MCP tool discovery scan: enqueue discovery for any
                        // connection whose refresh interval is due. Same
                        // 10-minute bucket as the token refresh above, and the
                        // same caveat — this dedup is per-process, so every
                        // node runs the scan. That is fine: the scan only
                        // ENQUEUES, and the per-connection lease inside the
                        // discovery job is what keeps the writes single-fire.
                        let mcp_dedup = format!("mcp-discovery-check:{}", now_secs / 600);
                        let mcp_job =
                            raisin_storage::jobs::JobType::McpDiscoveryCheck { tenant_id: None };
                        let mcp_job_id = raisin_storage::jobs::JobId::new();
                        let mcp_context = raisin_storage::jobs::JobContext {
                            tenant_id: "_system".to_string(),
                            repo_id: String::new(),
                            branch: String::new(),
                            workspace_id: String::new(),
                            revision: raisin_hlc::HLC::now(),
                            metadata: std::collections::HashMap::new(),
                        };
                        if let Err(e) = job_data_store_for_loop.put(&mcp_job_id, &mcp_context) {
                            tracing::warn!(error = %e, "Failed to store McpDiscoveryCheck job context");
                        } else if let Err(e) = registry_for_loop
                            .register_job_with_id_idempotent(
                                mcp_job_id,
                                mcp_job,
                                "_system".to_string(),
                                mcp_dedup,
                                None,
                            )
                            .await
                        {
                            tracing::warn!(error = %e, "Failed to register McpDiscoveryCheck job");
                        }

                        // Virtual-mount sync check: scan every ~60s for mounts
                        // that are due and enqueue per-mount sync jobs. Dedup
                        // by the wall-clock-minute bucket so overlapping ticks
                        // (or cluster nodes) collapse to a single scan job; the
                        // per-mount syncs it enqueues are themselves idempotent.
                        let check_bucket = now_secs / 60;
                        let vmount_check_job =
                            raisin_storage::jobs::JobType::VirtualMountSyncCheck {
                                tenant_id: None,
                                repo_id: None,
                            };
                        let vmount_check_id = raisin_storage::jobs::JobId::new();
                        let vmount_check_ctx = raisin_storage::jobs::JobContext {
                            tenant_id: "_system".to_string(),
                            repo_id: String::new(),
                            branch: String::new(),
                            workspace_id: String::new(),
                            revision: raisin_hlc::HLC::now(),
                            metadata: std::collections::HashMap::new(),
                        };
                        if let Err(e) =
                            job_data_store_for_loop.put(&vmount_check_id, &vmount_check_ctx)
                        {
                            tracing::warn!(error = %e, "Failed to store VirtualMountSyncCheck job context");
                        } else if let Err(e) = registry_for_loop
                            .register_job_with_id_idempotent(
                                vmount_check_id,
                                vmount_check_job,
                                "_system".to_string(),
                                format!("vmount-check:{check_bucket}"),
                                None,
                            )
                            .await
                        {
                            tracing::warn!(error = %e, "Failed to register VirtualMountSyncCheck job");
                        }

                        // Virtual-mount push-subscription renewal: enqueue at
                        // most once per 30-minute window (dedup key collapses
                        // every 60s tick inside the window). The handler renews
                        // provider subscriptions expiring within a day, keeping
                        // "on new email / on calendar change" push alive.
                        let renew_bucket = now_secs / (30 * 60);
                        let renew_job =
                            raisin_storage::jobs::JobType::VirtualMountSubscriptionRenew {
                                tenant_id: None,
                            };
                        let renew_id = raisin_storage::jobs::JobId::new();
                        let renew_ctx = raisin_storage::jobs::JobContext {
                            tenant_id: "_system".to_string(),
                            repo_id: String::new(),
                            branch: String::new(),
                            workspace_id: String::new(),
                            revision: raisin_hlc::HLC::now(),
                            metadata: std::collections::HashMap::new(),
                        };
                        if let Err(e) = job_data_store_for_loop.put(&renew_id, &renew_ctx) {
                            tracing::warn!(error = %e, "Failed to store VirtualMountSubscriptionRenew job context");
                        } else if let Err(e) = registry_for_loop
                            .register_job_with_id_idempotent(
                                renew_id,
                                renew_job,
                                "_system".to_string(),
                                format!("vmount-sub-renew:{renew_bucket}"),
                                None,
                            )
                            .await
                        {
                            tracing::warn!(error = %e, "Failed to register VirtualMountSubscriptionRenew job");
                        }

                        // Virtual-mount write reconcile: walk each writable
                        // mount's revisions from its writeback watermark, at
                        // most once every 5 minutes. This is the DURABLE half
                        // of change detection — the only path that can see a
                        // local delete, since a deleted node is gone from the
                        // workspace and from the sync index and is recoverable
                        // only from its MVCC pre-image, which garbage
                        // collection eventually takes.
                        //
                        // Same dedup caveat as the scans above: the registry's
                        // dedup map is per-process, so every cluster node runs
                        // its own copy. Here that is NOT harmless — this job
                        // writes mount state — so the handler takes a per-mount
                        // lease before touching anything.
                        let reconcile_bucket = now_secs / 300;
                        let reconcile_job =
                            raisin_storage::jobs::JobType::VirtualMountWriteReconcile {
                                tenant_id: None,
                                repo_id: None,
                            };
                        let reconcile_id = raisin_storage::jobs::JobId::new();
                        let reconcile_ctx = raisin_storage::jobs::JobContext {
                            tenant_id: "_system".to_string(),
                            repo_id: String::new(),
                            branch: String::new(),
                            workspace_id: String::new(),
                            revision: raisin_hlc::HLC::now(),
                            metadata: std::collections::HashMap::new(),
                        };
                        if let Err(e) = job_data_store_for_loop.put(&reconcile_id, &reconcile_ctx) {
                            tracing::warn!(error = %e, "Failed to store VirtualMountWriteReconcile job context");
                        } else if let Err(e) = registry_for_loop
                            .register_job_with_id_idempotent(
                                reconcile_id,
                                reconcile_job,
                                "_system".to_string(),
                                format!("vmount-write-reconcile:{reconcile_bucket}"),
                                None,
                            )
                            .await
                        {
                            tracing::warn!(error = %e, "Failed to register VirtualMountWriteReconcile job");
                        }

                        // Calendar occurrence projection: expand every series
                        // master over a rolling window into concrete, indexed
                        // occurrence nodes. Without it a date-range query over a
                        // calendar silently misses every recurring event, since
                        // a master's own `start_utc` is only its FIRST
                        // occurrence.
                        //
                        // Enqueued every 5 minutes; the handler's own
                        // per-workspace lease is what makes the actual rebuild
                        // happen at most once per interval per cluster, because
                        // the registry's dedup map is per-process.
                        let expand_bucket = now_secs / 300;
                        let expand_job = raisin_storage::jobs::JobType::CalendarOccurrenceRebuild {
                            tenant_id: None,
                            repo_id: None,
                        };
                        let expand_id = raisin_storage::jobs::JobId::new();
                        let expand_ctx = raisin_storage::jobs::JobContext {
                            tenant_id: "_system".to_string(),
                            repo_id: String::new(),
                            branch: String::new(),
                            workspace_id: String::new(),
                            revision: raisin_hlc::HLC::now(),
                            metadata: std::collections::HashMap::new(),
                        };
                        if let Err(e) = job_data_store_for_loop.put(&expand_id, &expand_ctx) {
                            tracing::warn!(error = %e, "Failed to store CalendarOccurrenceRebuild job context");
                        } else if let Err(e) = registry_for_loop
                            .register_job_with_id_idempotent(
                                expand_id,
                                expand_job,
                                "_system".to_string(),
                                format!("calendar-occurrence-rebuild:{expand_bucket}"),
                                None,
                            )
                            .await
                        {
                            tracing::warn!(error = %e, "Failed to register CalendarOccurrenceRebuild job");
                        }
                    }
                });
                tracing::info!("Scheduled trigger enqueuer started (60s interval)");
            }

            // Flow wait sweeper: backstop that catches waiting flow
            // instances whose timeout-check job was lost (queueing failure,
            // cleanup, clock skew). Idempotent - duplicates are deduped by
            // the job registry and check_flow_timeout no-ops on undue waits.
            flow_sweeper::spawn_flow_wait_sweeper(storage.clone(), 120);

            tracing::info!(
                "Total: {} dispatcher workers across {} runtime threads",
                total_workers,
                total_threads,
            );

            // Keep runtimes alive for the server's lifetime.
            // Dropping them would terminate all worker tasks.
            (
                Some(pool),
                Some(token),
                Some(rt_runtime),
                Some(bg_runtime),
                Some(sys_runtime),
            )
        } else {
            tracing::info!("Background jobs disabled in config");
            (None, None, None, None, None)
        }
    };

    // ========================================================================
    // Embedding and HNSW management
    // ========================================================================

    #[cfg(feature = "storage-rocksdb")]
    let (embedding_storage, embedding_job_store) = {
        let (emb_storage, emb_job_store) =
            startup::indexing::init_embedding_storage(db_clone.clone());
        (Some(emb_storage), Some(emb_job_store))
    };

    #[cfg(feature = "storage-rocksdb")]
    let hnsw_management = {
        let management = startup::indexing::init_hnsw_management(
            hnsw_engine.clone().unwrap(),
            embedding_storage.clone().unwrap(),
            &storage,
        );
        Some(management)
    };

    // ========================================================================
    // HTTP router assembly
    // ========================================================================

    // Persistent audit store under RocksDB, so a node's audit history survives a
    // restart; the memory backend has no DB handle and keeps the process-local
    // store. One instance is shared by the HTTP router, the WS transport, and the
    // event-bus writer below.
    #[cfg(feature = "storage-rocksdb")]
    let audit_repo = Arc::new(storage.audit_repository());
    #[cfg(not(feature = "storage-rocksdb"))]
    let audit_repo = Arc::new(InMemoryAuditRepo::default());
    let audit_adapter = Arc::new(RepoAuditAdapter::new(audit_repo.clone()));

    // Audit logging is driven by a single event-bus subscriber: every write path
    // (node API, SQL DML, functions) commits through the transaction layer, which
    // emits a NodeEvent per change. This handler turns those into audit-log
    // entries for NodeTypes marked `auditable` — so audit is uniform across all
    // transports and write surfaces, not just the node API.
    storage
        .event_bus()
        .subscribe(Arc::new(raisin_core::AuditEventHandler::new(
            audit_repo.clone(),
            storage.clone(),
        )));

    let ws_svc = Arc::new(WorkspaceService::new(storage.clone()));

    // Clone engines for pgwire before they're consumed by HTTP router
    #[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
    let pgwire_indexing_engine = indexing_engine.clone();
    #[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
    let pgwire_hnsw_engine = hnsw_engine.clone();

    // Clone engines for WS transport before they're consumed by HTTP router
    #[cfg(feature = "storage-rocksdb")]
    let ws_indexing_engine = indexing_engine.clone();
    #[cfg(feature = "storage-rocksdb")]
    let ws_hnsw_engine = hnsw_engine.clone();

    // Shutdown signal for every long-lived connection.
    //
    // `axum::serve(...).with_graceful_shutdown(...)` waits for every OPEN
    // connection, and two kinds never end on their own: an upgraded WebSocket,
    // and an SSE response (whose body ends only when its stream does). Either
    // one — a single idle Studio tab, a single open admin console — held the
    // drain until systemd's timeout fired SIGKILL, and a SIGKILL leaves
    // RocksDB's WAL unflushed for the next boot to replay (observed: ~350 MB vs
    // 192 KB after a clean stop). Cancelling this token tells WebSocket tasks to
    // finish in-flight work and close, and tells SSE streams to end.
    //
    // Created here, before the routers, because both the HTTP `AppState` and the
    // management state need it.
    //
    // Separate from the job system's `shutdown_token`, which is `None` when
    // background jobs are disabled — this one must always fire.
    let conn_shutdown = tokio_util::sync::CancellationToken::new();

    // Install the operator's outbound-MCP settings before anything can execute
    // a proxy function. Process-global, so every execution path — HTTP, WS,
    // jobs, flows — enforces the SAME egress policy the HTTP layer applies when
    // a connection is saved.
    raisin_functions::configure_mcp_client(server_config.mcp_client.clone());

    // Operator-named platform hooks for `raisin.platform.hook`. Same
    // process-global shape as the MCP client settings, for the same reason.
    raisin_functions::configure_platform_hooks(
        server_config
            .platform
            .hooks
            .iter()
            .map(|(name, h)| {
                (
                    name.clone(),
                    raisin_functions::PlatformHook {
                        url: h.url.clone(),
                        token: h.token.clone(),
                        token_env: h.token_env.clone(),
                        token_header: h.token_header.clone(),
                        timeout_ms: h.timeout_ms,
                    },
                )
            })
            .collect(),
    );

    // Hold notification streams open to remote MCP servers that announce tool
    // changes, so a tool added upstream appears in seconds rather than at the
    // next refresh interval.
    //
    // A supervised task rather than a job, deliberately: every JobType carries a
    // hard wall-clock deadline (600s is the largest in the enum) and the
    // watchdog reaps on runtime, so a stream-holding job would be severed on a
    // fixed cycle forever. Spawned HERE because this is the only point where the
    // storage, the lock manager, `conn_shutdown` and the installed MCP settings
    // are all live at once — and outside the `background_jobs_enabled` block,
    // where `conn_shutdown` does not yet exist.
    #[cfg(feature = "storage-rocksdb")]
    let mcp_listener = raisin_rocksdb::mcp_listener::spawn(
        raisin_rocksdb::mcp_listener::ListenerConfig {
            storage: storage.clone(),
            lock_manager: lock_manager.clone(),
            replication_enabled: server_config.replication_enabled,
            node_id: server_config
                .cluster_node_id
                .clone()
                .unwrap_or_else(|| nanoid::nanoid!(8)),
            http: raisin_functions::shared_http_client(),
        },
        conn_shutdown.clone(),
    );

    let (api_router, app_state) = http::router_with_bin_and_audit(
        storage.clone(),
        ws_svc.clone(),
        bin.clone(),
        audit_repo.clone(),
        audit_adapter,
        server_config.anonymous_enabled,
        server_config.dev_mode,
        env!("CARGO_PKG_VERSION").to_string(),
        &server_config.cors_allowed_origins,
        #[cfg(feature = "storage-rocksdb")]
        indexing_engine,
        #[cfg(feature = "storage-rocksdb")]
        tantivy_management,
        #[cfg(feature = "storage-rocksdb")]
        embedding_storage,
        #[cfg(feature = "storage-rocksdb")]
        embedding_job_store,
        #[cfg(feature = "storage-rocksdb")]
        hnsw_engine.clone(),
        #[cfg(feature = "storage-rocksdb")]
        hnsw_management,
        #[cfg(feature = "storage-rocksdb")]
        Some(storage.clone()),
        #[cfg(feature = "storage-rocksdb")]
        Some(auth_service.clone()),
        Some(schema_stats_cache.clone()),
        lock_manager.clone(),
        Some(server_config.mcp_client.clone()),
    );

    // Let the HTTP layer's long-lived responses (SSE) observe the shutdown
    // signal. Installed here rather than passed as yet another positional
    // argument, and before the management router is built below — that router
    // reads the token back out of `app_state`.
    app_state.configure_shutdown_token(conn_shutdown.clone());

    // Hand the HTTP layer the same definition stack the startup resync used, so
    // the pending/apply endpoints and the overlay-reload endpoint all operate on
    // one view of what this server offers.
    app_state
        .configure_system_definitions(
            server_config.system_definitions.clone(),
            server_config.data_dir.clone(),
        )
        .await;

    let admin_router = Router::new()
        .route("/admin", get(admin_ui::serve_admin_ui))
        .route("/admin/{*path}", get(admin_ui::serve_admin_ui));

    #[cfg(any(feature = "storage-rocksdb", feature = "storage-rocksdb"))]
    let management_router = {
        let graph_cache_state = if storage.config().background_jobs_enabled {
            use raisin_rocksdb::graph::GraphComputeConfig;
            let config = GraphComputeConfig::default();
            tracing::info!(
                "Starting graph cache background task (interval: {:?})",
                config.check_interval
            );
            Some(management::graph_cache::start_graph_cache_background_task(
                storage.clone(),
                config,
            ))
        } else {
            tracing::info!("Graph cache background task disabled (background_jobs_enabled=false)");
            None
        };

        management::management_router(
            storage.clone(),
            monitoring.clone(),
            graph_cache_state,
            hnsw_engine.clone(),
            app_state.clone(),
        )
    };

    // ========================================================================
    // WebSocket transport
    // ========================================================================

    // WebSocket connections observe `conn_shutdown` (created above, shared with
    // the HTTP/SSE layer): each task finishes the requests it has already
    // started, sends a 1001 Close frame and returns.
    #[cfg(feature = "websocket")]
    let app = {
        tracing::info!("Initializing WebSocket transport...");

        // Same reader as the auth service and the preflight — a fourth copy of
        // the placeholder literal here is how the WS transport would end up
        // accepting a secret the rest of the process had already rejected.
        let jwt_secret = raisin_crypto::jwt_secret()
            .unwrap_or_else(|| raisin_crypto::INSECURE_JWT_DEFAULT.to_string());

        let require_auth = std::env::var("WS_REQUIRE_AUTH")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);

        let max_concurrent_ops = std::env::var("WS_MAX_CONCURRENT_OPS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(100);

        let global_concurrency_limit = std::env::var("WS_GLOBAL_CONCURRENCY_LIMIT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1000);

        let ws_config = WsConfig {
            max_concurrent_ops,
            initial_credits: 100,
            jwt_secret,
            require_auth,
            global_concurrency_limit: Some(global_concurrency_limit),
            anonymous_enabled: server_config.anonymous_enabled,
            dev_mode: server_config.dev_mode,
            shutdown_grace: std::time::Duration::from_secs(
                std::env::var("WS_SHUTDOWN_GRACE_SECS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(5),
            ),
        };

        let connection = Arc::new(raisin_core::RaisinConnection::with_storage(storage.clone()));
        let ws_state = Arc::new(
            WsState::new(
                storage.clone(),
                connection,
                ws_svc.clone(),
                bin.clone(),
                ws_config,
                #[cfg(feature = "storage-rocksdb")]
                Some(auth_service.clone()),
                #[cfg(feature = "storage-rocksdb")]
                Some(storage.clone()),
                #[cfg(feature = "storage-rocksdb")]
                ws_indexing_engine,
                #[cfg(feature = "storage-rocksdb")]
                ws_hnsw_engine,
                Some(schema_stats_cache.clone()),
                lock_manager.clone(),
                audit_repo.clone(),
            )
            .with_shutdown(conn_shutdown.clone()),
        );

        let ws_router = Router::new()
            // Operator / multi-tenant form (explicit tenant in path)
            .route("/sys/{tenant_id}", any(websocket_handler))
            .route("/sys/{tenant_id}/{repository}", any(websocket_handler))
            // Tenant-less form for public clients; tenant resolved from the
            // x-tenant-id header on the upgrade request, falling back to
            // "default" (mirrors HTTP ensure_tenant_middleware semantics)
            .route("/ws", any(websocket_handler_tenantless))
            .route("/ws/{repository}", any(websocket_handler_tenantless))
            .with_state(ws_state);

        tracing::info!("WebSocket transport initialized");
        if require_auth {
            tracing::info!("WebSocket authentication: REQUIRED");
        } else {
            tracing::warn!(
                "WebSocket authentication: DISABLED (set WS_REQUIRE_AUTH=true for production)"
            );
        }

        #[cfg(any(feature = "storage-rocksdb", feature = "storage-rocksdb"))]
        {
            api_router
                .merge(admin_router)
                .merge(management_router)
                .merge(ws_router)
        }

        #[cfg(not(any(feature = "storage-rocksdb", feature = "storage-rocksdb")))]
        {
            api_router.merge(admin_router).merge(ws_router)
        }
    };

    #[cfg(not(feature = "websocket"))]
    let app = {
        #[cfg(any(feature = "storage-rocksdb", feature = "storage-rocksdb"))]
        {
            api_router.merge(admin_router).merge(management_router)
        }

        #[cfg(not(any(feature = "storage-rocksdb", feature = "storage-rocksdb")))]
        {
            api_router.merge(admin_router)
        }
    };

    // ========================================================================
    // PostgreSQL wire protocol server
    // ========================================================================

    #[cfg(all(feature = "pgwire", feature = "storage-rocksdb"))]
    let _pgwire_handle = {
        if server_config.pgwire_enabled {
            tracing::info!("Initializing PostgreSQL wire protocol server...");

            let handle = startup::pgwire::start_pgwire_server(
                storage.clone(),
                auth_service.clone(),
                pgwire_indexing_engine,
                pgwire_hnsw_engine,
                Some(schema_stats_cache.clone()),
                lock_manager.clone(),
                &server_config.pgwire_bind_address,
                server_config.pgwire_port,
                server_config.pgwire_max_connections,
            );

            tracing::info!(
                "PostgreSQL wire protocol server listening on {}:{}",
                server_config.pgwire_bind_address,
                server_config.pgwire_port
            );
            tracing::info!(
                "PostgreSQL wire protocol max connections: {}",
                server_config.pgwire_max_connections
            );

            Some(handle)
        } else {
            tracing::info!("PostgreSQL wire protocol server disabled");
            None
        }
    };

    // ========================================================================
    // Start HTTP server
    // ========================================================================

    let addr: std::net::SocketAddr =
        format!("{}:{}", server_config.bind_address, server_config.port)
            .parse()
            .expect("Invalid bind address or port");

    tracing::info!("listening on http://{addr}");
    tracing::info!("admin console available at http://{addr}/admin");
    #[cfg(any(feature = "storage-rocksdb", feature = "storage-rocksdb"))]
    tracing::info!("management API available at http://{addr}/management");
    #[cfg(feature = "websocket")]
    tracing::info!("WebSocket endpoint available at ws://{addr}/ws");
    #[cfg(feature = "storage-rocksdb")]
    tracing::warn!("SQL Query API enabled at /api/sql/{{repo}} - FOR DEVELOPMENT ONLY!");

    #[cfg(feature = "storage-rocksdb")]
    if let (Some(ref node_id), Some(repl_port)) = (
        &server_config.cluster_node_id,
        server_config.replication_port,
    ) {
        tracing::info!("Cluster replication enabled:");
        tracing::info!("   Node ID: {}", node_id);
        tracing::info!("   Replication port: {}", repl_port);
        tracing::info!("   Peers: {}", server_config.replication_peers.len());
    } else {
        tracing::info!("Running in STANDALONE mode (replication disabled)");
    }

    let listener = TcpListener::bind(addr)
        .await
        .expect("Failed to bind TCP listener to address");

    // The batch aggregator's shutdown token is `Some` when the job
    // system is enabled; `None` in the trimmed configurations that
    // skip it. Either way, we still want to react to SIGTERM so axum
    // exits cleanly — pass `None` through and `shutdown_signal` skips
    // the aggregator cancel.
    #[cfg(feature = "storage-rocksdb")]
    let batch_token = shutdown_token.clone();
    #[cfg(not(feature = "storage-rocksdb"))]
    let batch_token: Option<tokio_util::sync::CancellationToken> = None;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(batch_token, conn_shutdown))
        .await
        .expect("Failed to serve HTTP application");

    // Wait for the MCP listener to release its leases before the DB closes.
    //
    // This is the FIRST background task this process joins — every other one is
    // a detached `tokio::spawn` that simply stops existing when the runtime
    // dies. Joining matters here because `LockGuard` has no `Drop` impl: a lease
    // that is not explicitly released leaves that connection dark on every other
    // node until its TTL lapses. Bounded, so a wedged stream cannot hold the
    // whole shutdown open — the TTL is the backstop if this times out.
    #[cfg(feature = "storage-rocksdb")]
    if let Some(handle) = mcp_listener {
        match tokio::time::timeout(std::time::Duration::from_secs(5), handle).await {
            Ok(Ok(())) => tracing::debug!("MCP listener stopped cleanly"),
            Ok(Err(e)) => tracing::warn!(error = %e, "MCP listener task failed"),
            Err(_) => tracing::warn!(
                "MCP listener did not stop within 5s; its leases will expire on their TTL"
            ),
        }
    }

    // Close RocksDB explicitly.
    //
    // Nothing else does. `Arc<DB>` handles are cloned into services, jobs and
    // caches all over the process, so returning from `main` does NOT reliably
    // run the DB's destructor — which is the only thing that would flush the
    // memtables and the WAL. Without this, even a *clean* SIGTERM can leave a
    // large WAL for the next boot to replay. Blocking, so it goes on a
    // blocking thread rather than a runtime worker.
    #[cfg(feature = "storage-rocksdb")]
    {
        let storage_for_close = storage.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || storage_for_close.flush_and_close())
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
        {
            tracing::warn!(error = %e, "RocksDB flush on shutdown reported an error");
        }
    }

    // Tear down the dedicated job-pool runtimes OFF the async context.
    //
    // `main` runs inside the `#[tokio::main]` runtime, and these three runtimes
    // are owned here, so letting them drop normally at the end of `main` panics:
    //   "Cannot drop a runtime in a context where blocking is not allowed."
    // A runtime's `Drop` blocks to join its worker threads, and blocking is
    // forbidden inside an async context. `shutdown_background()` hands that
    // teardown to a separate thread, so SIGTERM/Ctrl-C shutdown exits cleanly.
    #[cfg(feature = "storage-rocksdb")]
    {
        for rt in [_rt_runtime, _bg_runtime, _sys_runtime]
            .into_iter()
            .flatten()
        {
            rt.shutdown_background();
        }
    }
}

/// Wait for SIGINT (Ctrl-C) or SIGTERM, then release everything that would
/// otherwise hold the drain open:
///
/// 1. `conn_shutdown` — WebSocket connections AND SSE streams. `axum::serve`'s
///    graceful shutdown waits for every open connection, and neither an
///    upgraded WebSocket nor an SSE response ever ends on its own, so without
///    this ONE idle browser tab keeps the process alive until the supervisor
///    SIGKILLs it (and a SIGKILL means an unflushed WAL that the next boot has
///    to replay). Each WebSocket finishes the requests it has already started,
///    then sends a 1001 Close frame and returns; each SSE stream ends, which
///    completes its response body normally so the browser reconnects rather
///    than reporting an error.
/// 2. The batch aggregator's shutdown token, so its background flush task
///    drains the in-memory pending map (and the persisted CF) before exit.
///
/// SIGKILL still bypasses all of this — that's why the `pending_batch_ops`
/// column family exists as a second line of defense (see
/// `batch_aggregator::persistence`).
async fn shutdown_signal(
    batch_shutdown: Option<tokio_util::sync::CancellationToken>,
    conn_shutdown: tokio_util::sync::CancellationToken,
) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("SIGINT received; initiating graceful shutdown");
        }
        _ = terminate => {
            tracing::info!("SIGTERM received; initiating graceful shutdown");
        }
    }

    // Do this FIRST. axum's drain waits on every open connection, and an
    // upgraded WebSocket never ends on its own — so unless the sockets are
    // told to close, the drain below never completes and the process is
    // eventually SIGKILLed with an unflushed WAL.
    conn_shutdown.cancel();
    tracing::info!("Signalled WebSocket connections to drain in-flight work and close");

    if let Some(token) = batch_shutdown {
        token.cancel();
        tracing::info!(
            "Cancelled batch aggregator; waiting for axum to stop accepting connections"
        );
    } else {
        tracing::info!("No batch aggregator running; waiting for axum to drain");
    }
}
