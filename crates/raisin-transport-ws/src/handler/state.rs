// SPDX-License-Identifier: BSL-1.1

//! WebSocket shared state and upgrade handler.

use axum::{
    extract::{ws::WebSocketUpgrade, State},
    http::HeaderMap,
    response::IntoResponse,
};
use std::sync::Arc;

use crate::{auth::JwtAuthService, event_handler::WsEventHandler, registry::ConnectionRegistry};

use super::config::{WsConfig, WsPathParams};
use super::socket::handle_socket;

/// WebSocket upgrade handler
///
/// This is the entry point for WebSocket connections.
/// URL format: /sys/{tenant_id} or /sys/{tenant_id}/{repository}
pub async fn websocket_handler<S, B>(
    axum::extract::Path(params): axum::extract::Path<WsPathParams>,
    State(state): State<Arc<WsState<S, B>>>,
    ws: WebSocketUpgrade,
    headers: HeaderMap,
) -> impl IntoResponse
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let tenant_id = params.tenant_id.clone();
    let repository = params.repository.clone();

    tracing::debug!(
        "WebSocket connection request - tenant: {:?}, repository: {:?}",
        tenant_id,
        repository
    );

    tracing::info!(
        "WebSocket upgrade for tenant: {}, repository: {:?}",
        tenant_id,
        repository
    );

    // Extract authentication token from headers (if present)
    let token = JwtAuthService::extract_token_from_headers(&headers);

    // Upgrade the connection
    ws.on_upgrade(move |socket| handle_socket(socket, state, token, tenant_id, repository))
}

/// WebSocket state shared across all connections
pub struct WsState<S, B>
where
    S: raisin_storage::Storage,
    B: raisin_binary::BinaryStorage,
{
    /// Storage instance
    pub storage: Arc<S>,

    /// RaisinDB connection
    pub connection: Arc<raisin_core::RaisinConnection<S>>,

    /// Workspace service
    pub ws_svc: Arc<raisin_core::WorkspaceService<S>>,

    /// Binary storage
    pub bin: Arc<B>,

    /// Configuration
    pub config: WsConfig,

    /// JWT authentication service
    pub auth_service: Arc<JwtAuthService>,

    /// Global operation semaphore
    pub global_semaphore: Option<Arc<tokio::sync::Semaphore>>,

    /// Event bus for subscriptions
    pub event_bus: Arc<dyn raisin_events::EventBus>,

    /// Connection registry for event forwarding
    pub connection_registry: Arc<ConnectionRegistry>,

    /// RocksDB auth service (only available with storage-rocksdb feature)
    #[cfg(feature = "storage-rocksdb")]
    pub rocksdb_auth_service: Option<Arc<raisin_rocksdb::AuthService>>,

    /// RocksDB storage for tenant auth config lookups (only available with storage-rocksdb feature)
    #[cfg(feature = "storage-rocksdb")]
    pub rocksdb_storage: Option<Arc<raisin_rocksdb::RocksDBStorage>>,

    /// Tantivy indexing engine for SQL query execution (only available with storage-rocksdb feature)
    #[cfg(feature = "storage-rocksdb")]
    pub indexing_engine: Option<Arc<raisin_indexer::TantivyIndexingEngine>>,

    /// HNSW vector indexing engine (only available with storage-rocksdb feature)
    #[cfg(feature = "storage-rocksdb")]
    pub hnsw_engine: Option<Arc<raisin_hnsw::HnswIndexingEngine>>,

    /// Shared schema stats cache for data-driven selectivity estimation
    pub schema_stats_cache: Option<raisin_core::SharedSchemaStatsCache>,
}

impl<S, B> WsState<S, B>
where
    S: raisin_storage::Storage + Send + Sync + 'static,
    B: raisin_binary::BinaryStorage,
{
    pub fn new(
        storage: Arc<S>,
        connection: Arc<raisin_core::RaisinConnection<S>>,
        ws_svc: Arc<raisin_core::WorkspaceService<S>>,
        bin: Arc<B>,
        config: WsConfig,
        #[cfg(feature = "storage-rocksdb")] rocksdb_auth_service: Option<
            Arc<raisin_rocksdb::AuthService>,
        >,
        #[cfg(feature = "storage-rocksdb")] rocksdb_storage: Option<
            Arc<raisin_rocksdb::RocksDBStorage>,
        >,
        #[cfg(feature = "storage-rocksdb")] indexing_engine: Option<
            Arc<raisin_indexer::TantivyIndexingEngine>,
        >,
        #[cfg(feature = "storage-rocksdb")] hnsw_engine: Option<
            Arc<raisin_hnsw::HnswIndexingEngine>,
        >,
        schema_stats_cache: Option<raisin_core::SharedSchemaStatsCache>,
    ) -> Self {
        let event_bus = storage.event_bus();
        let auth_service = Arc::new(JwtAuthService::new(&config.jwt_secret));

        let global_semaphore = config
            .global_concurrency_limit
            .map(|limit| Arc::new(tokio::sync::Semaphore::new(limit)));

        // Create connection registry for tracking active connections
        let connection_registry = Arc::new(ConnectionRegistry::new());

        // Register the WebSocket event handler with the event bus
        // Pass storage for RLS evaluation on node events
        let ws_event_handler = Arc::new(WsEventHandler::new(
            Arc::clone(&connection_registry),
            Arc::clone(&storage),
        ));
        event_bus.subscribe(ws_event_handler);

        Self {
            storage,
            connection,
            ws_svc,
            bin,
            config,
            auth_service,
            global_semaphore,
            event_bus,
            connection_registry,
            #[cfg(feature = "storage-rocksdb")]
            rocksdb_auth_service,
            #[cfg(feature = "storage-rocksdb")]
            rocksdb_storage,
            #[cfg(feature = "storage-rocksdb")]
            indexing_engine,
            #[cfg(feature = "storage-rocksdb")]
            hnsw_engine,
            schema_stats_cache,
        }
    }

    /// Check if anonymous access is enabled for a tenant/repo context.
    ///
    /// Priority (highest to lowest):
    /// 1. Repo-level config (node in raisin:system workspace)
    /// 2. Access control stewardship config (where admin console saves it)
    /// 3. Tenant-level config (TenantAuthConfig in RocksDB)
    /// 4. Global config (server config file anonymous_enabled setting)
    #[cfg(feature = "storage-rocksdb")]
    pub async fn is_anonymous_enabled(&self, tenant_id: &str, repo_id: &str) -> bool {
        use raisin_storage::{NodeRepository, Storage, StorageScope};

        // If we have RocksDB storage, check tenant config
        if let Some(ref rocksdb) = self.rocksdb_storage {
            // 1. Check repo-level config from system workspace node (highest priority)
            let repo_config_path = format!("/config/repos/{}", repo_id);
            let repo_config_node = Storage::nodes(rocksdb.as_ref())
                .get_by_path(
                    StorageScope::new(tenant_id, repo_id, "main", "raisin:system"),
                    &repo_config_path,
                    None,
                )
                .await
                .ok()
                .flatten();

            if let Some(node) = repo_config_node {
                if node.node_type == "raisin:RepoAuthConfig" {
                    if let Some(raisin_models::nodes::properties::PropertyValue::Boolean(enabled)) =
                        node.properties.get("anonymous_enabled")
                    {
                        tracing::debug!(
                            tenant_id = %tenant_id,
                            repo_id = %repo_id,
                            anonymous_enabled = %enabled,
                            "Anonymous access from repo config"
                        );
                        return *enabled;
                    }
                }
            }

            // 2. Check access_control stewardship config (where admin console saves anonymous_enabled)
            let stewardship_node = Storage::nodes(rocksdb.as_ref())
                .get_by_path(
                    StorageScope::new(tenant_id, repo_id, "main", "raisin:access_control"),
                    "/config/stewardship",
                    None,
                )
                .await
                .ok()
                .flatten();

            if let Some(node) = stewardship_node {
                if let Some(raisin_models::nodes::properties::PropertyValue::Boolean(enabled)) =
                    node.properties.get("anonymous_enabled")
                {
                    tracing::debug!(
                        tenant_id = %tenant_id,
                        repo_id = %repo_id,
                        anonymous_enabled = %enabled,
                        "Anonymous access from access_control stewardship config"
                    );
                    return *enabled;
                }
            }

            // 3. Check tenant-level config from RocksDB column family
            let tenant_config = rocksdb
                .tenant_auth_config_repository()
                .get_config(tenant_id)
                .await
                .ok()
                .flatten();

            if let Some(config) = tenant_config {
                tracing::debug!(
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    anonymous_enabled = %config.anonymous_enabled,
                    "Anonymous access from tenant config"
                );
                return config.anonymous_enabled;
            }

            // 4. Fall back to global config when no tenant/repo config exists
            tracing::debug!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                anonymous_enabled = %self.config.anonymous_enabled,
                "Anonymous access from global config (no tenant config found)"
            );
            return self.config.anonymous_enabled;
        }

        // Fall back to global config
        self.config.anonymous_enabled
    }

    /// Check if anonymous access is enabled (non-RocksDB fallback).
    #[cfg(not(feature = "storage-rocksdb"))]
    pub async fn is_anonymous_enabled(&self, _tenant_id: &str, _repo_id: &str) -> bool {
        self.config.anonymous_enabled
    }
}
