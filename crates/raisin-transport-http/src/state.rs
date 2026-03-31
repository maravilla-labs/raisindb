// SPDX-License-Identifier: BSL-1.1

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use raisin_core::{NodeService, RaisinConnection, RepoAuditAdapter, TtlCache, WorkspaceService};
use raisin_models::auth::AuthContext;

use crate::upload_processors::UploadProcessorRegistry;
use raisin_audit::InMemoryAuditRepo;
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::RocksDBStorage;
#[cfg(not(feature = "storage-rocksdb"))]
use raisin_storage_memory::InMemoryStorage;

#[cfg(not(feature = "s3"))]
use raisin_binary::FilesystemBinaryStorage;
#[cfg(feature = "s3")]
use raisin_binary::S3BinaryStorage;

#[cfg(feature = "storage-rocksdb")]
use raisin_indexer::{TantivyIndexingEngine, TantivyManagement};

#[cfg(feature = "storage-rocksdb")]
use raisin_hnsw::HnswIndexingEngine;
#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::{HnswManagement, RocksDBEmbeddingJobStore, RocksDBEmbeddingStorage};

#[cfg(feature = "s3")]
pub(crate) type Bin = S3BinaryStorage;
#[cfg(not(feature = "s3"))]
pub(crate) type Bin = FilesystemBinaryStorage;

#[cfg(feature = "storage-rocksdb")]
pub(crate) type Store = RocksDBStorage;
#[cfg(not(feature = "storage-rocksdb"))]
pub(crate) type Store = InMemoryStorage;

// Use InMemoryAuditRepo for both storage backends for now
// TODO: Implement RocksDB-backed audit repository
type AuditRepo = InMemoryAuditRepo;

#[derive(Clone)]
pub struct AppState {
    pub(crate) storage: Arc<Store>,
    pub(crate) connection: Arc<RaisinConnection<Store>>,
    pub(crate) ws_svc: Arc<WorkspaceService<Store>>,
    pub(crate) bin: Arc<Bin>,
    pub(crate) audit: Arc<AuditRepo>,
    pub(crate) audit_adapter: Arc<RepoAuditAdapter<AuditRepo>>,
    /// Registry of upload processors for node-type-specific upload handling
    pub(crate) upload_processors: Arc<UploadProcessorRegistry>,
    /// Whether anonymous access is enabled globally.
    /// When true, unauthenticated HTTP requests will be auto-authenticated
    /// as the "anonymous" user with the "anonymous" role permissions.
    pub(crate) anonymous_enabled: bool,
    /// Development mode — allows insecure defaults for secrets.
    pub(crate) dev_mode: bool,
    /// Global CORS allowed origins from server config (TOML).
    /// Used as fallback when tenant/repo-level CORS is not configured.
    pub(crate) cors_allowed_origins: Vec<String>,
    /// CORS origin cache with 60s TTL.
    /// Keyed by `{tenant}/{repo}` or `{tenant}/__all__` for repo-less routes.
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) cors_cache: Arc<TtlCache<Vec<String>>>,
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) indexing_engine: Option<Arc<TantivyIndexingEngine>>,
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) tantivy_management: Option<Arc<TantivyManagement>>,
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) embedding_storage: Option<Arc<RocksDBEmbeddingStorage>>,
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) embedding_job_store: Option<Arc<RocksDBEmbeddingJobStore>>,
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) hnsw_engine: Option<Arc<HnswIndexingEngine>>,
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) hnsw_management: Option<Arc<HnswManagement>>,
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) rocksdb_storage: Option<Arc<raisin_rocksdb::RocksDBStorage>>,
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) auth_service: Option<Arc<raisin_rocksdb::AuthService>>,
}

impl AppState {
    /// Get access to the RaisinConnection for transaction API
    pub(crate) fn connection(&self) -> &Arc<RaisinConnection<Store>> {
        &self.connection
    }

    /// Get access to the underlying storage for NodeType operations
    pub(crate) fn storage(&self) -> &Arc<Store> {
        &self.storage
    }

    /// Get access to the underlying RocksDB instance (RocksDB only)
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) fn rocksdb(&self) -> Option<Arc<rocksdb::DB>> {
        self.rocksdb_storage.as_ref().map(|s| Arc::clone(s.db()))
    }

    /// Create a workspace-scoped NodeService for the given workspace
    /// This is a helper for handlers that extract workspace from the path
    #[deprecated(note = "Use node_service_for_context instead")]
    pub(crate) fn node_service_for_workspace(&self, workspace_id: &str) -> NodeService<Store> {
        // For single-tenant mode, use default tenant/repo/branch
        // TODO: In multi-tenant mode, extract these from headers/path
        NodeService::new_with_context(
            self.storage.clone(),
            "default".to_string(), // tenant_id
            "main".to_string(),    // repo_id
            "main".to_string(),    // branch
            workspace_id.to_string(),
        )
        .with_audit(self.audit_adapter.clone())
    }

    /// Create a NodeService with full repository-first context
    /// tenant_id should come from middleware (auth/headers)
    /// repository, branch, and workspace come from URL path
    /// auth should be extracted from request extensions (set by auth middleware)
    pub(crate) fn node_service_for_context(
        &self,
        tenant_id: &str,
        repository: &str,
        branch: &str,
        workspace_id: &str,
        auth: Option<AuthContext>,
    ) -> NodeService<Store> {
        let svc = NodeService::new_with_context(
            self.storage.clone(),
            tenant_id.to_string(),
            repository.to_string(),
            branch.to_string(),
            workspace_id.to_string(),
        )
        .with_audit(self.audit_adapter.clone());

        // Apply auth context for RLS filtering if provided
        match auth {
            Some(ctx) => svc.with_auth(ctx),
            None => svc,
        }
    }

    /// Get master encryption key for API key encryption.
    ///
    /// Reads from `RAISIN_MASTER_KEY` (or legacy `EMBEDDING_MASTER_KEY`) env var
    /// as a 64-character hex string.  In dev-mode, falls back to an all-zero key
    /// with a warning.  In production mode, returns an error if not set.
    pub(crate) fn get_master_key(&self) -> raisin_error::Result<[u8; 32]> {
        let hex_key = std::env::var("RAISIN_MASTER_KEY")
            .or_else(|_| std::env::var("EMBEDDING_MASTER_KEY"))
            .ok();

        match hex_key {
            Some(hex) => {
                let bytes = hex::decode(&hex).map_err(|e| {
                    raisin_error::Error::Validation(format!(
                        "RAISIN_MASTER_KEY is not valid hex: {e}"
                    ))
                })?;
                let key: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
                    raisin_error::Error::Validation(format!(
                        "RAISIN_MASTER_KEY must be 32 bytes (64 hex chars), got {} bytes",
                        v.len()
                    ))
                })?;
                Ok(key)
            }
            None if self.dev_mode => {
                tracing::warn!(
                    "RAISIN_MASTER_KEY not set — using insecure all-zero key (dev-mode)"
                );
                Ok([0u8; 32])
            }
            None => Err(raisin_error::Error::Validation(
                "RAISIN_MASTER_KEY (or EMBEDDING_MASTER_KEY) must be set. \
                 Use --dev-mode to allow insecure defaults."
                    .to_string(),
            )),
        }
    }

    /// Get signing secret for HMAC-signed asset URLs.
    ///
    /// Reads from `RAISINDB_SIGNING_SECRET` environment variable.
    /// In dev-mode, falls back to a hard-coded development key with a warning.
    /// In production mode, returns an error if not set.
    pub(crate) fn get_signing_secret(&self) -> raisin_error::Result<Vec<u8>> {
        match std::env::var("RAISINDB_SIGNING_SECRET") {
            Ok(s) => Ok(s.into_bytes()),
            Err(_) if self.dev_mode => {
                tracing::warn!(
                    "RAISINDB_SIGNING_SECRET not set — using insecure fallback (dev-mode)"
                );
                Ok(b"raisindb-dev-signing-secret-key!".to_vec())
            }
            Err(_) => Err(raisin_error::Error::Validation(
                "RAISINDB_SIGNING_SECRET must be set. \
                 Use --dev-mode to allow insecure defaults."
                    .to_string(),
            )),
        }
    }

    /// Get access to the authentication service (RocksDB only)
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) fn auth_service(&self) -> Option<&Arc<raisin_rocksdb::AuthService>> {
        self.auth_service.as_ref()
    }

    /// Check if anonymous access is enabled (DEPRECATED)
    ///
    /// This returns the server-level default setting.
    /// For per-tenant/repo anonymous access, use `is_anonymous_enabled_for_context()`
    /// from the middleware which checks TenantAuthConfig and RepoAuthConfig.
    #[deprecated(note = "Use is_anonymous_enabled_for_context() from middleware instead")]
    pub(crate) fn is_anonymous_enabled(&self) -> bool {
        self.anonymous_enabled
    }
}

// This router function is kept for backward compatibility with tests
// Production code uses router_with_bin_and_audit() directly
#[cfg(not(feature = "s3"))]
pub fn router(storage: Arc<Store>) -> Router {
    let bin: Arc<Bin> = Arc::new(FilesystemBinaryStorage::new(
        "./.data/uploads",
        Some("/files".into()),
    ));

    // Create audit repo - currently using in-memory for all backends
    let audit: Arc<AuditRepo> = Arc::new(InMemoryAuditRepo::default());

    let adapter = Arc::new(RepoAuditAdapter::new(audit.clone()));
    let ws_svc = Arc::new(WorkspaceService::new(storage.clone()));
    // Default to anonymous disabled for test router
    // Production uses router_with_bin_and_audit() with explicit config
    let anonymous_enabled = std::env::var("HTTP_ANONYMOUS_ENABLED")
        .ok()
        .and_then(|v| v.parse::<bool>().ok())
        .unwrap_or(false);
    let (router, _state) = router_with_bin_and_audit(
        storage,
        ws_svc,
        bin,
        audit,
        adapter,
        anonymous_enabled,
        false, // dev_mode disabled for test router
        &[],   // No CORS for test router
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None,
        #[cfg(feature = "storage-rocksdb")]
        None, // auth_service
    );
    router
}

pub fn router_with_bin_and_audit(
    storage: Arc<Store>,
    ws_svc: Arc<WorkspaceService<Store>>,
    bin: Arc<Bin>,
    audit: Arc<AuditRepo>,
    audit_adapter: Arc<RepoAuditAdapter<AuditRepo>>,
    anonymous_enabled: bool,
    dev_mode: bool,
    cors_allowed_origins: &[String],
    #[cfg(feature = "storage-rocksdb")] indexing_engine: Option<Arc<TantivyIndexingEngine>>,
    #[cfg(feature = "storage-rocksdb")] tantivy_management: Option<Arc<TantivyManagement>>,
    #[cfg(feature = "storage-rocksdb")] embedding_storage: Option<Arc<RocksDBEmbeddingStorage>>,
    #[cfg(feature = "storage-rocksdb")] embedding_job_store: Option<Arc<RocksDBEmbeddingJobStore>>,
    #[cfg(feature = "storage-rocksdb")] hnsw_engine: Option<Arc<HnswIndexingEngine>>,
    #[cfg(feature = "storage-rocksdb")] hnsw_management: Option<Arc<HnswManagement>>,
    #[cfg(feature = "storage-rocksdb")] rocksdb_storage: Option<
        Arc<raisin_rocksdb::RocksDBStorage>,
    >,
    #[cfg(feature = "storage-rocksdb")] auth_service: Option<Arc<raisin_rocksdb::AuthService>>,
) -> (Router, AppState) {
    let connection = Arc::new(RaisinConnection::with_storage(storage.clone()));

    // Create upload processor registry with built-in processors
    let upload_processors = Arc::new(UploadProcessorRegistry::new());

    let state = AppState {
        storage,
        connection,
        ws_svc,
        bin,
        audit,
        audit_adapter,
        upload_processors,
        anonymous_enabled,
        dev_mode,
        cors_allowed_origins: cors_allowed_origins.to_vec(),
        #[cfg(feature = "storage-rocksdb")]
        cors_cache: Arc::new(TtlCache::new(std::time::Duration::from_secs(60))),
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
        rocksdb_storage,
        #[cfg(feature = "storage-rocksdb")]
        auth_service,
    };

    // NOTE: Global CorsLayer has been removed in favor of unified_cors_middleware
    // which implements hierarchical CORS resolution: Repo → Tenant → Global
    // The cors_allowed_origins are now stored in AppState for use by the middleware
    let router = axum::Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(crate::routes::routes(state.clone()));

    (router, state)
}

/// Helper function to extract RocksDB from AppState (RocksDB only)
#[cfg(feature = "storage-rocksdb")]
pub(crate) fn get_rocksdb_from_state(state: &AppState) -> raisin_error::Result<Arc<rocksdb::DB>> {
    state.rocksdb().ok_or_else(|| {
        raisin_error::Error::Backend("RocksDB not available in AppState".to_string())
    })
}
