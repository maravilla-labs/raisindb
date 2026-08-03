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
    /// Caller-INDEPENDENT half of an MCP server's tool set, keyed by
    /// `{tenant}\0{repo}\0{branch}\0{slug}` — a bounded key, like
    /// `cors_cache`, never a request-controlled path.
    ///
    /// Holds only what is safe to hand to ANY caller: the parsed descriptor and
    /// the resolved tool definitions. Scope filtering stays per-request, in
    /// `visible_descriptors` and again in `handle_tools_call`. This was rebuilt
    /// from storage on every single request — `1 + W` SQL scans materializing
    /// every `raisin:Function` node — before the JSON-RPC method was even read.
    ///
    /// TTL is a backstop; the real invalidation is event-driven, so an edited
    /// server or function shows up on the next request.
    pub(crate) mcp_plan_cache: Arc<TtlCache<Arc<raisin_mcp::McpServerPlan>>>,
    /// Permission resolution, cached (5-min TTL, keyed by
    /// `tenant:repo:branch:identity:<id>`).
    ///
    /// Held on `AppState` because the cache lives INSIDE the service — building
    /// one per request, as every call site used to, resolves nothing from cache
    /// and throws the entry away. Resolution is not cheap: a PROPERTY_INDEX
    /// prefix scan plus a full node load to find the user, then one lookup per
    /// group, then role inheritance, then a SECOND pass over the effective
    /// roles to collect permissions. An MCP request paid that twice — once in
    /// `optional_auth_middleware`'s anonymous fallback and once in
    /// `resolve_mcp_auth`.
    ///
    /// The TTL means a revoked role can stay effective for up to 5 minutes;
    /// `invalidate_user` / `invalidate_branch` exist for the paths that know.
    pub(crate) permission_service: Arc<raisin_core::CachedPermissionService<Store>>,
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
    /// `/register`, and discovery endpoints.
    ///
    /// Clients and refresh tokens are persisted (and replicated) through
    /// [`OAuthStore`]; authorization codes are sealed rather than stored, so
    /// any node can redeem one another node issued.
    #[cfg(feature = "storage-rocksdb")]
    pub(crate) oauth_server: Arc<raisin_auth::authserver::AuthorizationServer<OAuthStore>>,
    /// Live system-definition stack. Defaults to embedded-only; the server
    /// replaces it at startup via [`AppState::configure_system_definitions`].
    pub(crate) definitions: Arc<tokio::sync::RwLock<SystemDefinitionsState>>,
    /// Server-wide shutdown signal, fired first thing on SIGTERM/SIGINT.
    ///
    /// Empty unless the server installs one via
    /// [`AppState::configure_shutdown_token`], so every other constructor —
    /// tests included — keeps working and simply never cancels.
    ///
    /// Long-lived responses (SSE) MUST observe this. `axum::serve`'s graceful
    /// shutdown waits for every OPEN connection, and an SSE response ends only
    /// when its stream does, so one open admin-console tab holds the drain
    /// exactly as an idle WebSocket did — until systemd's `TimeoutStopSec`
    /// turns into SIGKILL, and a SIGKILL skips the RocksDB WAL flush.
    ///
    /// Held in a `OnceLock` behind an `Arc` on purpose: the state is cloned
    /// into the router at construction time, but the token is only created
    /// once the server is about to serve, so a plain field could never be
    /// installed after the fact.
    pub(crate) shutdown: Arc<std::sync::OnceLock<tokio_util::sync::CancellationToken>>,
}

/// Await a server shutdown signal, or never resolve if there is none.
///
/// The "never" arm is what keeps a token-less `AppState` (tests, embedded
/// users) behaving exactly as before: the stream is simply never truncated.
///
/// Boxed so the resulting `TakeUntil` stays `Unpin` for an `Unpin` stream —
/// otherwise every caller has to pin it by hand. One allocation per long-lived
/// connection is not worth optimising.
pub fn shutdown_or_never(
    token: Option<tokio_util::sync::CancellationToken>,
) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(async move {
        match token {
            Some(token) => token.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    })
}

/// The store backing the OAuth authorization server.
///
/// Durable, because RFC 7591 clients MUST outlive the process: an MCP host
/// (ChatGPT, Claude, Cursor) caches the `client_id` it was issued indefinitely,
/// so a store that empties on restart answers every subsequent `/authorize`
/// with `invalid_client: unknown client_id '…'` and the only user-visible
/// remedy is deleting and re-adding the connector.
///
/// `raisin_auth`'s `InMemoryAuthServerStore` remains available for that crate's
/// own tests, but is deliberately not reachable from here — the authorization
/// server only exists under `storage-rocksdb` in the first place, so there is
/// no configuration in which the volatile store would be the right choice.
#[cfg(feature = "storage-rocksdb")]
pub(crate) type OAuthStore = raisin_rocksdb::RocksDbOAuthStore;

impl AppState {
    /// Get access to the RaisinConnection for transaction API
    pub(crate) fn connection(&self) -> &Arc<RaisinConnection<Store>> {
        &self.connection
    }

    /// Get access to the underlying storage for NodeType operations
    pub fn storage(&self) -> &Arc<Store> {
        &self.storage
    }

    /// Install the process-wide shutdown token.
    ///
    /// Called once at startup, after the router is built — the state is shared
    /// by `Arc`, so the token is visible to every handler that already holds a
    /// clone. Idempotent: a second call is ignored.
    pub fn configure_shutdown_token(&self, token: tokio_util::sync::CancellationToken) {
        let _ = self.shutdown.set(token);
    }

    /// The process-wide shutdown token, if one was installed.
    pub fn shutdown_token(&self) -> Option<tokio_util::sync::CancellationToken> {
        self.shutdown.get().cloned()
    }

    /// A future that resolves when the server starts shutting down.
    ///
    /// Suitable for `StreamExt::take_until`: when no token was installed it
    /// never resolves, so the stream is unaffected.
    pub fn shutdown_signal(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        shutdown_or_never(self.shutdown_token())
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

    // Back the authorization server with RocksDB whenever we have it. Clients
    // registered via RFC 7591 MUST outlive the process: an MCP host caches its
    // `client_id` indefinitely, so an in-memory store turns every restart into
    // `invalid_client` for every previously-connected client.
    // Derived from `storage` rather than the optional `rocksdb_storage`
    // parameter on purpose: several callers (the test router included) pass
    // `None` there, and durability here must not depend on a caller
    // remembering to thread a handle through.
    #[cfg(feature = "storage-rocksdb")]
    let oauth_server = {
        // Authorization codes are sealed with the deployment master key rather
        // than stored, so any node can redeem a code any other node issued.
        // Without a configured key we fall back to a per-process random one:
        // that keeps dev servers and tests working, but codes then die with the
        // process and never cross nodes — hence the warning.
        let configured = match raisin_crypto::master_key_with_embedding_fallback() {
            Ok(key) => key,
            Err(e) => {
                // A present-but-malformed key is an operator error. This
                // constructor is infallible, so surface it loudly and fall back
                // rather than serving with a key we cannot trust.
                tracing::error!(
                    error = %e,
                    "RAISIN_MASTER_KEY is set but invalid; OAuth codes will use an ephemeral key"
                );
                None
            }
        };
        let codec = match configured {
            Some(key) => raisin_auth::authserver::SealedCodeCodec::new(&key),
            None => {
                tracing::warn!(
                    "RAISIN_MASTER_KEY is not set: OAuth authorization codes will be sealed \
                     with an ephemeral per-process key. In-flight logins will fail after a \
                     restart, and a multi-node deployment will fail every token exchange \
                     that lands on a different node than the one that issued the code."
                );
                raisin_auth::authserver::SealedCodeCodec::ephemeral()
            }
        };

        // One lock manager, always. The shared configured one when `[locks]` is
        // enabled — which is what makes replay detection cluster-wide — else a
        // process-local one, so the single-node guarantee holds with no config.
        let locks: raisin_locks::LockManagerHandle = lock_manager.clone().unwrap_or_else(|| {
            Arc::new(raisin_locks::InProcessLockManager::with_reaper(
                std::time::Duration::from_secs(30),
            ))
        });

        Arc::new(raisin_auth::authserver::AuthorizationServer::new(
            Arc::new(
                raisin_rocksdb::RocksDbOAuthStore::new(Arc::clone(storage.db()))
                    .with_capture(Arc::clone(storage.operation_capture())),
            ),
            Arc::new(codec),
            locks,
        ))
    };

    // Clone the handle before `storage` moves into the struct: the permission
    // cache must be ONE instance for the process, not one per request.
    let storage_for_permissions = Arc::clone(&storage);

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
        mcp_plan_cache: Arc::new(TtlCache::new(std::time::Duration::from_secs(300))),
        permission_service: Arc::new(raisin_core::CachedPermissionService::with_default_ttl(
            storage_for_permissions,
        )),
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
        // Installed later by the server via `configure_shutdown_token`; every
        // other caller leaves it empty and never cancels.
        shutdown: Arc::new(std::sync::OnceLock::new()),
    };

    // Keep the cached MCP plans honest. A package author editing a function and
    // not seeing it in `tools/list` would be a far worse bug than the latency
    // this cache removes, so the entries are dropped on the write rather than
    // waiting out the TTL.
    {
        use raisin_storage::Storage as _;
        state.storage.event_bus().subscribe(Arc::new(
            crate::handlers::mcp::plan_cache::McpPlanCacheInvalidator::new(
                state.mcp_plan_cache.clone(),
            ),
        ));
    }

    // NOTE: Global CorsLayer has been removed in favor of unified_cors_middleware
    // which implements hierarchical CORS resolution: Repo → Tenant → Global
    // The cors_allowed_origins are now stored in AppState for use by the middleware
    let router = axum::Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(crate::routes::routes(state.clone()));

    (router, state)
}

#[cfg(test)]
mod shutdown_tests {
    use super::shutdown_or_never;
    use futures::StreamExt;
    use tokio_util::sync::CancellationToken;

    /// The composition every SSE handler uses: a stream that would otherwise
    /// run forever must END when the token fires — ending the response body
    /// normally, which is what releases axum's graceful drain.
    ///
    /// (`AppState` itself needs a live storage backend to construct, so this
    /// exercises the helper the handlers call rather than a full router.)
    #[tokio::test]
    async fn stream_ends_when_token_is_cancelled() {
        let token = CancellationToken::new();
        let (tx, rx) = tokio::sync::mpsc::channel::<u32>(8);

        let mut stream = tokio_stream::wrappers::ReceiverStream::new(rx)
            .take_until(shutdown_or_never(Some(token.clone())));

        tx.send(1).await.unwrap();
        assert_eq!(stream.next().await, Some(1));

        token.cancel();

        // The sender is still alive and the channel still has items queued —
        // only the token ends this. Anything else would keep the connection
        // open through the drain.
        tx.send(2).await.unwrap();
        assert_eq!(stream.next().await, None, "stream must end on cancellation");
    }

    /// No token installed (tests, embedded use) must behave exactly as before:
    /// the stream is never truncated.
    #[tokio::test]
    async fn stream_is_unaffected_without_a_token() {
        let (tx, rx) = tokio::sync::mpsc::channel::<u32>(8);
        let mut stream =
            tokio_stream::wrappers::ReceiverStream::new(rx).take_until(shutdown_or_never(None));

        tx.send(7).await.unwrap();
        assert_eq!(stream.next().await, Some(7));

        // Still open: only dropping the sender ends it.
        drop(tx);
        assert_eq!(stream.next().await, None);
    }
}

/// Helper function to extract RocksDB from AppState (RocksDB only)
#[cfg(feature = "storage-rocksdb")]
pub(crate) fn get_rocksdb_from_state(state: &AppState) -> raisin_error::Result<Arc<rocksdb::DB>> {
    state.rocksdb().ok_or_else(|| {
        raisin_error::Error::Backend("RocksDB not available in AppState".to_string())
    })
}
