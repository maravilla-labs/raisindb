//  TODO(v0.2): Clean up unused code and deprecated usages
#![allow(deprecated)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unexpected_cfgs)]

mod admin_ui;
mod admin_user_init_handler;
#[cfg(feature = "storage-rocksdb")]
mod builtin_package_init_handler;
mod config;
mod deps_setup;
mod management;
#[cfg(feature = "storage-rocksdb")]
mod migrations;
mod nodetype_init_handler;
mod sse;
mod startup;
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
use raisin_transport_ws::{websocket_handler, WsConfig, WsState};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    // Parse command-line arguments and merge configuration sources
    let cli_config = startup::ServerConfig::parse();
    let server_config = cli_config.merge().expect("Failed to load configuration");

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("RaisinDB Server starting...");
    tracing::info!("  HTTP Port: {}", server_config.port);
    tracing::info!("  Data Directory: {}", server_config.data_dir);

    // ========================================================================
    // Dev-mode banner & production secret validation
    // ========================================================================

    if server_config.dev_mode {
        tracing::warn!("============================================================");
        tracing::warn!("  DEV-MODE ENABLED — insecure defaults are allowed.");
        tracing::warn!("  Do NOT use --dev-mode in production!");
        tracing::warn!("============================================================");
    } else {
        // In production mode, required secrets must be set
        let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_default();
        if jwt_secret.is_empty() || jwt_secret == "default_jwt_secret_change_in_production" {
            tracing::error!(
                "JWT_SECRET is not set or uses the insecure default. \
                 Set a strong JWT_SECRET or use --dev-mode for development."
            );
            std::process::exit(1);
        }

        if std::env::var("RAISINDB_SIGNING_SECRET").is_err() {
            tracing::error!(
                "RAISINDB_SIGNING_SECRET is not set. \
                 Set a random 32+ byte string or use --dev-mode for development."
            );
            std::process::exit(1);
        }

        if std::env::var("RAISIN_MASTER_KEY").is_err()
            && std::env::var("EMBEDDING_MASTER_KEY").is_err()
        {
            tracing::error!(
                "RAISIN_MASTER_KEY (or EMBEDDING_MASTER_KEY) is not set. \
                 Provide a 64-char hex key or use --dev-mode for development."
            );
            std::process::exit(1);
        }
    }

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
        let engine = startup::indexing::init_hnsw_engine(hnsw_path);
        Some(engine)
    };

    // ========================================================================
    // Binary storage
    // ========================================================================

    #[cfg(feature = "s3")]
    let bin = startup::binary::init_binary_storage().await;
    #[cfg(not(feature = "s3"))]
    let bin = startup::binary::init_binary_storage(&server_config.data_dir);

    #[cfg(feature = "storage-rocksdb")]
    startup::binary::register_builtin_package_handler(&storage, &bin).await;

    // ========================================================================
    // Job system initialization
    // ========================================================================

    #[cfg(feature = "storage-rocksdb")]
    let (_worker_pool, _shutdown_token, _rt_runtime, _bg_runtime, _sys_runtime) = {
        if storage.config().background_jobs_enabled {
            tracing::info!("Initializing unified job system...");

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
                http_client: reqwest::Client::new(),
                ai_config_store: Some(Arc::new(storage.tenant_ai_config_repository())),
                job_registry: Some(storage.job_registry().clone()),
                job_data_store: Some(storage.job_data_store().clone()),
            });

            let execution_callbacks = ExecutionProvider::create_callbacks_with_deps(
                execution_deps.clone(),
                FunctionExecutionConfig::default(),
            );
            tracing::info!("Using production execution callbacks with full dependencies");

            use raisin_functions::execution::create_flow_callbacks;
            let flow_callbacks = create_flow_callbacks(execution_deps.clone());
            tracing::info!("Created flow instance execution callbacks");

            let scheduled_trigger_finder: Option<raisin_rocksdb::ScheduledTriggerFinderCallback> =
                None;

            let binary_storage_callback =
                startup::jobs::create_binary_storage_callback(bin.clone());

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

    let audit_repo = Arc::new(raisin_audit::InMemoryAuditRepo::default());
    let audit_adapter = Arc::new(RepoAuditAdapter::new(audit_repo.clone()));
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

    let (api_router, app_state) = http::router_with_bin_and_audit(
        storage.clone(),
        ws_svc.clone(),
        bin.clone(),
        audit_repo.clone(),
        audit_adapter,
        server_config.anonymous_enabled,
        server_config.dev_mode,
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
        hnsw_engine,
        #[cfg(feature = "storage-rocksdb")]
        hnsw_management,
        #[cfg(feature = "storage-rocksdb")]
        Some(storage.clone()),
        #[cfg(feature = "storage-rocksdb")]
        Some(auth_service.clone()),
    );

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
            app_state.clone(),
        )
    };

    // ========================================================================
    // WebSocket transport
    // ========================================================================

    #[cfg(feature = "websocket")]
    let app = {
        tracing::info!("Initializing WebSocket transport...");

        let jwt_secret = std::env::var("JWT_SECRET")
            .unwrap_or_else(|_| "default_jwt_secret_change_in_production".to_string());

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
        };

        let connection = Arc::new(raisin_core::RaisinConnection::with_storage(storage.clone()));
        let ws_state = Arc::new(WsState::new(
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
        ));

        let ws_router = Router::new()
            .route("/sys/{tenant_id}", any(websocket_handler))
            .route("/sys/{tenant_id}/{repository}", any(websocket_handler))
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
    axum::serve(listener, app)
        .await
        .expect("Failed to serve HTTP application");
}
