// SPDX-License-Identifier: BSL-1.1

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use raisin_core::{
    NodeService, RaisinConnection, RepoAuditAdapter, SharedSchemaStatsCache, TtlCache,
    WorkspaceService,
};
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

// Audit-log store, chosen by storage backend like `Store` above. The
// `AuditRepository` trait is not dyn-safe (RPITIT), so this alias — not a trait
// object — is the swap point. RocksDB persists audit history across restarts;
// the memory backend has no `DB` handle and keeps the process-local store.
#[cfg(feature = "storage-rocksdb")]
pub(crate) type AuditRepo = raisin_rocksdb::RocksDBAuditRepo;
#[cfg(not(feature = "storage-rocksdb"))]
pub(crate) type AuditRepo = InMemoryAuditRepo;

/// The server's live system-definition stack.
///
/// Held behind a lock so the overlay directory can be re-read at runtime
/// (`POST /api/management/system-definitions/reload`) without a restart: an
/// operator drops a corrected YAML on the box, reloads, and the change shows up
/// as a pending system update. The config and data dir travel with the resolver
/// because both the reload and any registry fetch need to know where the overlay
/// lives.
pub struct SystemDefinitionsState {
    /// Currently resolved layers.
    pub resolver: Arc<raisin_core::definitions::DefinitionResolver>,
    /// The `[system_definitions]` section this resolver was built from.
    pub config: raisin_core::definitions::SystemDefinitionsConfig,
    /// Server data directory, for resolving a defaulted overlay path.
    pub data_dir: String,
}

impl Default for SystemDefinitionsState {
    fn default() -> Self {
        Self {
            resolver: Arc::new(raisin_core::definitions::DefinitionResolver::embedded_only()),
            config: raisin_core::definitions::SystemDefinitionsConfig::default(),
            data_dir: String::new(),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) storage: Arc<Store>,
    pub(crate) connection: Arc<RaisinConnection<Store>>,
    pub(crate) ws_svc: Arc<WorkspaceService<Store>>,
    pub(crate) bin: Arc<Bin>,
    pub(crate) audit: Arc<AuditRepo>,
    /// Registry of upload processors for node-type-specific upload handling
    pub(crate) upload_processors: Arc<UploadProcessorRegistry>,
    /// Whether anonymous access is enabled globally.
    /// When true, unauthenticated HTTP requests will be auto-authenticated
    /// as the "anonymous" user with the "anonymous" role permissions.
    pub(crate) anonymous_enabled: bool,
    /// Development mode — allows insecure defaults for secrets.
    pub(crate) dev_mode: bool,
    /// Server version string (sourced from raisin-server's CARGO_PKG_VERSION at startup).
    /// Exposed to the admin SPA via the `/api/admin/bootstrap` endpoint.
    pub(crate) server_version: String,
    /// Snapshot of `RAISIN_SUPERADMIN_TOKEN` resolved once at startup. Held as
    /// `Arc<str>` so the bootstrap handler can hand out a refcount clone
    /// rather than reading `std::env::var` on every SPA boot.
    pub(crate) superadmin_token: Option<Arc<str>>,
    /// Global CORS allowed origins from server config (TOML).
    /// Used as fallback when tenant/repo-level CORS is not configured.
    pub(crate) cors_allowed_origins: Vec<String>,
    /// CORS origin cache with 60s TTL.
    /// Keyed by `{tenant}/{repo}` or `{tenant}/__all__` for repo-less routes.
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) cors_cache: Arc<TtlCache<Vec<String>>>,
    /// Per-workspace `raisin:StaticSiteFolder` set for the `/resources` gate
    /// (60s TTL). Keyed by `{tenant}\0{repo}\0{branch}\0{ws}` — a bounded key
    /// (like `cors_cache`), NOT the request path, so adversarial/unbounded path
    /// traffic cannot grow it. Resolved with system auth so the decision is
    /// principal-independent and safe to share across callers; the asset bytes
    /// themselves are still served RLS-scoped.
    pub(crate) static_site_cache:
        Arc<TtlCache<Arc<Vec<crate::handlers::static_site::StaticSiteEntry>>>>,
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
    /// Shared schema stats cache for data-driven query plan selectivity estimation
    pub(crate) schema_stats_cache: Option<SharedSchemaStatsCache>,
    /// Atomic locks / inventory manager. `None` when the subsystem is disabled.
    pub(crate) lock_manager: Option<raisin_locks::LockManagerHandle>,
    /// OAuth 2.1 authorization server backing the `/authorize`, `/token`,
    /// `/register`, and discovery endpoints. Holds registered clients and
    /// short-lived authorization codes in-process via the default store.
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) oauth_server: Arc<raisin_auth::authserver::AuthorizationServer<OAuthStore>>,
    /// Live system-definition stack. Defaults to embedded-only; the server
    /// replaces it at startup via [`AppState::configure_system_definitions`].
    pub(crate) definitions: Arc<tokio::sync::RwLock<SystemDefinitionsState>>,
}

/// The store backing the OAuth authorization server.
///
/// The in-memory implementation is a complete, real store suitable for
/// single-node servers, the CLI dev server, and tests. A managed deployment can
/// swap in a persistent implementation of the same store traits without
/// changing the protocol logic.
#[cfg(feature = "storage-rocksdb")]
pub(crate) type OAuthStore = raisin_auth::authserver::InMemoryAuthServerStore;

impl AppState {
    /// Get access to the RaisinConnection for transaction API
    pub(crate) fn connection(&self) -> &Arc<RaisinConnection<Store>> {
        &self.connection
    }

    /// Get access to the underlying storage for NodeType operations
    pub fn storage(&self) -> &Arc<Store> {
        &self.storage
    }

    /// Install the server's configured system-definition stack.
    ///
    /// Called once at startup, after the router is built: the state is shared
    /// by `Arc`, so replacing the contents here is visible to every handler.
    pub async fn configure_system_definitions(
        &self,
        config: raisin_core::definitions::SystemDefinitionsConfig,
        data_dir: impl Into<String>,
    ) {
        let data_dir = data_dir.into();
        let resolver = Arc::new(raisin_core::definitions::build_and_install_resolver(
            &config, &data_dir,
        ));
        let mut guard = self.definitions.write().await;
        *guard = SystemDefinitionsState {
            resolver,
            config,
            data_dir,
        };
    }

    /// The currently resolved definition stack.
    pub async fn definitions(&self) -> Arc<raisin_core::definitions::DefinitionResolver> {
        self.definitions.read().await.resolver.clone()
    }

    /// The `[system_definitions]` config and the resolved overlay directory.
    pub async fn system_definitions_config(
        &self,
    ) -> (
        raisin_core::definitions::SystemDefinitionsConfig,
        std::path::PathBuf,
    ) {
        let guard = self.definitions.read().await;
        let dir = guard.config.resolved_overlay_dir(&guard.data_dir);
        (guard.config.clone(), dir)
    }

    /// Re-read the overlay directory and rebuild the definition stack.
    ///
    /// This is the "no restart" half of out-of-band updates: files land on the
    /// box, this rebuilds the resolver, and the changed definitions surface as
    /// pending system updates. It does NOT write anything into any repository.
    pub async fn reload_system_definitions(
        &self,
    ) -> Arc<raisin_core::definitions::DefinitionResolver> {
        let mut guard = self.definitions.write().await;
        let resolver = Arc::new(raisin_core::definitions::build_and_install_resolver(
            &guard.config,
            &guard.data_dir,
        ));
        guard.resolver = resolver.clone();
        resolver
    }

    /// Get access to the underlying RocksDB instance (RocksDB only)
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) fn rocksdb(&self) -> Option<Arc<rocksdb::DB>> {
        self.rocksdb_storage.as_ref().map(|s| Arc::clone(s.db()))
    }

    /// Get the RocksDB storage handle (RocksDB only).
    ///
    /// Public because the operator surface in `raisin-server` constructs
    /// repositories directly instead of going through a handler in this crate.
    #[cfg(feature = "storage-rocksdb")]
    pub fn rocksdb_storage(&self) -> Option<&Arc<raisin_rocksdb::RocksDBStorage>> {
        self.rocksdb_storage.as_ref()
    }

    /// Create a workspace-scoped NodeService for the given workspace
    /// This is a helper for handlers that extract workspace from the path
    #[deprecated(note = "Use node_service_for_context instead")]
    pub(crate) fn node_service_for_workspace(&self, workspace_id: &str) -> NodeService<Store> {
        // For single-tenant mode, use default tenant/repo/branch
        // TODO: In multi-tenant mode, extract these from headers/path
        // Audit logging is handled globally by the event-bus AuditEventHandler
        // (covers node API, SQL DML, and functions uniformly), so the per-service
        // audit sink is not wired here.
        NodeService::new_with_context(
            self.storage.clone(),
            "default".to_string(), // tenant_id
            "main".to_string(),    // repo_id
            "main".to_string(),    // branch
            workspace_id.to_string(),
        )
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
        // Audit logging is handled globally by the event-bus AuditEventHandler;
        // the per-service audit sink is intentionally not wired here.
        let svc = NodeService::new_with_context(
            self.storage.clone(),
            tenant_id.to_string(),
            repository.to_string(),
            branch.to_string(),
            workspace_id.to_string(),
        );

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
        // Shared loader accepts RAISIN_MASTER_KEY with the legacy
        // EMBEDDING_MASTER_KEY fallback, returning Ok(None) when neither is set
        // and Err on an invalid value.
        match raisin_crypto::master_key_with_embedding_fallback()? {
            Some(key) => Ok(key),
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
    pub fn auth_service(&self) -> Option<&Arc<raisin_rocksdb::AuthService>> {
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

    // Audit repo, matching the backend: persistent under RocksDB, process-local
    // otherwise.
    #[cfg(feature = "storage-rocksdb")]
    let audit: Arc<AuditRepo> = Arc::new(storage.audit_repository());
    #[cfg(not(feature = "storage-rocksdb"))]
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
        false,                    // dev_mode disabled for test router
        "0.0.0-test".to_string(), // server_version placeholder for test router
        &[],                      // No CORS for test router
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
        None, // schema_stats_cache
        None, // lock_manager
    );
    router
}

pub fn router_with_bin_and_audit(
    storage: Arc<Store>,
    ws_svc: Arc<WorkspaceService<Store>>,
    bin: Arc<Bin>,
    audit: Arc<AuditRepo>,
    // Retained for signature compatibility; audit is now handled by the
    // event-bus AuditEventHandler rather than a per-service sink.
    _audit_adapter: Arc<RepoAuditAdapter<AuditRepo>>,
    anonymous_enabled: bool,
    dev_mode: bool,
    server_version: String,
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
    schema_stats_cache: Option<SharedSchemaStatsCache>,
    lock_manager: Option<raisin_locks::LockManagerHandle>,
) -> (Router, AppState) {
    let connection = Arc::new(RaisinConnection::with_storage(storage.clone()));

    // Create upload processor registry with built-in processors
    let upload_processors = Arc::new(UploadProcessorRegistry::new());

    // Resolve the superadmin token once at startup. We only retain it when
    // dev_mode is on; in production the bootstrap endpoint never surfaces it
    // so caching it would only widen the in-memory exposure.
    let superadmin_token: Option<Arc<str>> = if dev_mode {
        std::env::var("RAISIN_SUPERADMIN_TOKEN")
            .ok()
            .filter(|t| !t.is_empty())
            .map(Arc::<str>::from)
    } else {
        None
    };

    // The authorization server holds OAuth clients + codes in process. Codes are
    // single-use and short-lived; clients persist for the server's lifetime.
    #[cfg(feature = "storage-rocksdb")]
    let oauth_server = Arc::new(raisin_auth::authserver::AuthorizationServer::new(Arc::new(
        raisin_auth::authserver::InMemoryAuthServerStore::new(),
    )));

    let state = AppState {
        storage,
        connection,
        ws_svc,
        bin,
        audit,
        upload_processors,
        anonymous_enabled,
        dev_mode,
        server_version,
        superadmin_token,
        cors_allowed_origins: cors_allowed_origins.to_vec(),
        static_site_cache: Arc::new(TtlCache::new(std::time::Duration::from_secs(60))),
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
        schema_stats_cache,
        lock_manager,
        #[cfg(feature = "storage-rocksdb")]
        oauth_server,
        definitions: Arc::new(tokio::sync::RwLock::new(SystemDefinitionsState::default())),
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
