//! SQL Query Engine
//!
//! Provides a high-level API for executing SQL queries on RaisinDB storage.
//! Handles the complete pipeline: SQL -> Parse -> Analyze -> Optimize -> Plan -> Execute -> Results
//!
//! # Module Structure
//!
//! - `batch` - Batch SQL execution with async routing
//! - `branch` - Branch management statement execution
//! - `handlers` - Statement-type handlers (EXPLAIN, DML, DDL, Transaction, SHOW, SELECT)
//! - `helpers` - Compound index loading and node_type extraction
//! - `restore` - RESTORE statement execution

mod acl;
#[cfg(test)]
mod admin_statement_gate_tests;
mod ai_config;
mod batch;
mod branch;
pub mod catalog_cache;
#[cfg(test)]
mod embedding_config_reader_tests;
mod handlers;
pub(crate) mod helpers;
mod phase_timing;
mod physical_cache;
mod prepared;
mod prepared_params;
mod query_exec;
mod restore;
mod spatial_admin;
#[cfg(test)]
mod statement_context_tests;
mod subquery_bind;

pub use batch::batch_requires_async;
pub use catalog_cache::{
    invalidate_all_workspace_catalogs, invalidate_workspace_catalog, workspace_catalog,
};
pub use helpers::invalidate_compound_index_cache;
pub use prepared::{invalidate_plan_cache, plan_cache_contains, plan_cache_stats};
pub use prepared_params::{template_cache_stats, ParamOutcome};
pub use query_exec::physical_plan_cache_hits;

use crate::physical_plan::executor::{execute_plan, ExecutionContext, RowStream};
use crate::physical_plan::IndexCatalog;
use raisin_context::RepositoryConfig;
use raisin_core::SharedSchemaStatsCache;
use raisin_embeddings::embedding_storage::EmbeddingStorage;
use raisin_embeddings::provider::EmbeddingProvider;
use raisin_error::Error;
use raisin_hnsw::HnswIndexingEngine;
use raisin_indexer::TantivyIndexingEngine;
use raisin_models::auth::AuthContext;
use raisin_sql::analyzer::{AnalyzedStatement, Analyzer, Catalog, StaticCatalog};
use raisin_sql::logical_plan::PlanBuilder;
use raisin_sql::optimizer::Optimizer;
use raisin_storage::{ArchetypeRepository, BranchRepository, NodeTypeRepository, Storage};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

/// How a parameter value is rendered as a SQL literal for `$n` substitution
/// (see [`QueryEngine::execute_with_params`]).
pub type ParamFormat = dyn Fn(&serde_json::Value) -> String + Send + Sync;

/// Callback type for registering async bulk SQL jobs
///
/// This callback is provided by the transport layer (HTTP/WS handlers) which has
/// access to RocksDB-specific job registry and data store. The callback receives:
/// - `sql`: The SQL batch to execute asynchronously
/// - `actor`: The user/actor who submitted the job
///
/// Returns the job ID string on success.
pub type JobRegistrarCallback = Arc<
    dyn Fn(String, String) -> Pin<Box<dyn Future<Output = Result<String, Error>> + Send>>
        + Send
        + Sync,
>;

/// Callback for async function invocation via SQL INVOKE().
/// Args: (function_path, input_json, optional_workspace) -> (execution_id, job_id)
pub type FunctionInvokeCallback = Arc<
    dyn Fn(
            String,            // function_path
            serde_json::Value, // input_json
            Option<String>,    // workspace
        ) -> Pin<Box<dyn Future<Output = Result<(String, String), Error>> + Send>>
        + Send
        + Sync,
>;

/// Callback for sync function invocation via SQL INVOKE_SYNC().
/// Args: (function_path, input_json, optional_workspace) -> result_json
pub type FunctionInvokeSyncCallback = Arc<
    dyn Fn(
            String,            // function_path
            serde_json::Value, // input_json
            Option<String>,    // workspace
        ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, Error>> + Send>>
        + Send
        + Sync,
>;

/// Callback type for registering async RESTORE TREE jobs
///
/// This callback is provided by the transport layer (HTTP/WS handlers) which has
/// access to RocksDB-specific job registry and data store. The callback receives:
/// - `node_id`: ID of the node to restore
/// - `node_path`: Path of the node to restore
/// - `revision_hlc`: HLC timestamp string to restore from
/// - `translations`: Optional list of translations to restore (None = all)
/// - `actor`: The user/actor who submitted the job
///
/// Returns the job ID string on success.
pub type RestoreTreeRegistrarCallback = Arc<
    dyn Fn(
            String,              // node_id
            String,              // node_path
            String,              // revision_hlc
            Option<Vec<String>>, // translations
            String,              // actor
        ) -> Pin<Box<dyn Future<Output = Result<String, Error>> + Send>>
        + Send
        + Sync,
>;

/// SQL Query Engine for RaisinDB
///
/// Provides a complete SQL execution pipeline with support for:
/// - Workspace-as-table queries (`SELECT FROM workspace_name`)
/// - Revision-aware queries (`WHERE __revision = 342`)
/// - Full-text search, prefix scans, property indexes
/// - Hierarchical path operations
pub struct QueryEngine<S: Storage> {
    pub(crate) storage: Arc<S>,
    pub(crate) indexing_engine: Option<Arc<TantivyIndexingEngine>>,
    pub(crate) hnsw_engine: Option<Arc<HnswIndexingEngine>>,
    pub(crate) embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    pub(crate) embedding_storage: Option<Arc<dyn EmbeddingStorage>>,
    pub(crate) catalog: Arc<dyn Catalog>,
    pub(crate) tenant_id: String,
    pub(crate) repo_id: String,
    /// Default branch (from constructor, typically repository's default_branch)
    pub(crate) branch: String,
    /// Session-level branch override (set by USE BRANCH / SET app.branch)
    pub(crate) session_branch: RwLock<Option<String>>,
    /// Local branch override (set by USE LOCAL BRANCH / SET LOCAL app.branch)
    pub(crate) local_branch: RwLock<Option<String>>,
    /// Pending session branch from USE BRANCH in current batch
    pub(crate) pending_session_branch: RwLock<Option<String>>,
    pub(crate) default_language: String,
    /// Active transaction context for BEGIN...COMMIT workflow
    pub(crate) transaction_context:
        Arc<RwLock<Option<Box<dyn raisin_storage::transactional::TransactionalContext>>>>,
    /// Optional callback for registering async bulk SQL jobs
    pub(crate) job_registrar: Option<JobRegistrarCallback>,
    /// Optional callback for registering async RESTORE TREE jobs
    pub(crate) restore_tree_registrar: Option<RestoreTreeRegistrarCallback>,
    /// Optional callback for async function invocation (INVOKE)
    pub(crate) function_invoke: Option<FunctionInvokeCallback>,
    /// Optional callback for sync function invocation (INVOKE_SYNC)
    pub(crate) function_invoke_sync: Option<FunctionInvokeSyncCallback>,
    /// Atomic lock / inventory manager for RAISIN_TRY_ACQUIRE/RAISIN_CLAIM/etc.
    pub(crate) lock_manager: Option<Arc<dyn raisin_locks::LockManager>>,
    /// Default actor for job registration
    pub(crate) default_actor: String,
    /// Repository configuration for locale-aware queries
    pub(crate) repository_config: Option<RepositoryConfig>,
    /// Authentication context for RLS filtering
    pub(crate) auth_context: Option<AuthContext>,
    /// Tenant embedding config store for AI config SQL statements
    pub(crate) embedding_config_store:
        Option<Arc<dyn raisin_embeddings::TenantEmbeddingConfigStore>>,
    /// Tenant AI config store, needed to resolve a `ai_provider_ref`.
    ///
    /// Without it, `TEST EMBEDDING CONNECTION` can only see the legacy fields
    /// — which is how a console-configured tenant (the console writes the
    /// unified ref) got "No API key configured for this tenant" from a config
    /// the embedding job resolved perfectly well.
    pub(crate) ai_config_store: Option<Arc<dyn raisin_embeddings::resolve::TenantAIConfigStore>>,
    /// Master key for API key encryption/decryption
    pub(crate) master_key: Option<[u8; 32]>,
    /// Shared schema stats cache for data-driven selectivity estimation
    pub(crate) schema_stats_cache: Option<SharedSchemaStatsCache>,
    /// `sql.batched_fetch` for every statement this engine runs.
    pub(crate) batched_fetch: bool,
}

/// The default nodes schema, built once for the process.
///
/// `QueryEngine::new` is almost always followed by `.with_catalog(...)`, so the
/// default used to be constructed (several hundred allocations across five
/// `TableDef`s) and thrown away on every single engine construction — which is
/// itself per query on every transport.
fn default_catalog() -> Arc<dyn Catalog> {
    static DEFAULT: std::sync::LazyLock<Arc<StaticCatalog>> =
        std::sync::LazyLock::new(|| Arc::new(StaticCatalog::default_nodes_schema()));
    DEFAULT.clone()
}

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Create a new query engine
    pub fn new(
        storage: Arc<S>,
        tenant_id: impl Into<String>,
        repo_id: impl Into<String>,
        branch: impl Into<String>,
    ) -> Self {
        Self {
            storage,
            indexing_engine: None,
            hnsw_engine: None,
            embedding_provider: None,
            embedding_storage: None,
            catalog: default_catalog(),
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            branch: branch.into(),
            session_branch: RwLock::new(None),
            local_branch: RwLock::new(None),
            pending_session_branch: RwLock::new(None),
            default_language: "en".to_string(),
            transaction_context: Arc::new(RwLock::new(None)),
            job_registrar: None,
            restore_tree_registrar: None,
            function_invoke: None,
            function_invoke_sync: None,
            lock_manager: None,
            default_actor: "anonymous".to_string(),
            repository_config: None,
            auth_context: None,
            embedding_config_store: None,
            ai_config_store: None,
            master_key: None,
            schema_stats_cache: None,
            batched_fetch: crate::physical_plan::executor::context::batched_fetch_default(),
        }
    }

    /// `sql.batched_fetch`: `false` makes index scans and RESOLVE read one node
    /// at a time instead of in batches (the rollback switch). The default comes
    /// from `RAISIN_SQL_BATCHED_FETCH` (on unless set to `0`/`false`/`off`).
    pub fn with_batched_fetch(mut self, batched: bool) -> Self {
        self.batched_fetch = batched;
        self
    }

    /// A fresh statement context carrying this engine's per-statement
    /// switches. EVERY SQL `ExecutionContext` is built here, so a new
    /// construction site cannot forget one (the scalar `SELECT RESOLVE(...)`
    /// path once kept the env default after `with_batched_fetch(false)`).
    pub(crate) fn new_statement_context(
        &self,
        branch: String,
        workspace: String,
    ) -> ExecutionContext<S> {
        let mut ctx = ExecutionContext::new(
            self.storage.clone(),
            self.tenant_id.clone(),
            self.repo_id.clone(),
            branch,
            workspace,
        );
        ctx.batched_fetch = self.batched_fetch;
        ctx
    }

    /// Set the default language for queries without explicit locale specification
    pub fn with_default_language(mut self, language: impl Into<String>) -> Self {
        self.default_language = language.into();
        self
    }

    /// Set a custom catalog for table schema resolution
    pub fn with_catalog(mut self, catalog: Arc<dyn Catalog>) -> Self {
        self.catalog = catalog;
        self
    }

    /// Enable full-text search with Tantivy indexing engine
    pub fn with_indexing_engine(mut self, engine: Arc<TantivyIndexingEngine>) -> Self {
        self.indexing_engine = Some(engine);
        self
    }

    /// Enable vector similarity search with HNSW indexing engine
    pub fn with_hnsw_engine(mut self, engine: Arc<HnswIndexingEngine>) -> Self {
        self.hnsw_engine = Some(engine);
        self
    }

    /// Set embedding provider for EMBEDDING() function evaluation
    pub fn with_embedding_provider(mut self, provider: Arc<dyn EmbeddingProvider>) -> Self {
        self.embedding_provider = Some(provider);
        self
    }

    /// Set embedding storage for reading embeddings from storage
    pub fn with_embedding_storage(mut self, storage: Arc<dyn EmbeddingStorage>) -> Self {
        self.embedding_storage = Some(storage);
        self
    }

    /// Set job registrar callback for async bulk SQL operations
    pub fn with_job_registrar(mut self, registrar: JobRegistrarCallback) -> Self {
        self.job_registrar = Some(registrar);
        self
    }

    /// Set restore tree job registrar callback for async RESTORE TREE operations
    pub fn with_restore_tree_registrar(mut self, registrar: RestoreTreeRegistrarCallback) -> Self {
        self.restore_tree_registrar = Some(registrar);
        self
    }

    /// Set the function invoke callback for async INVOKE() function
    pub fn with_function_invoke(mut self, cb: FunctionInvokeCallback) -> Self {
        self.function_invoke = Some(cb);
        self
    }

    /// Set the function invoke sync callback for INVOKE_SYNC() function
    /// Set the atomic lock / inventory manager (enables RAISIN_TRY_ACQUIRE etc.)
    pub fn with_lock_manager(mut self, manager: Arc<dyn raisin_locks::LockManager>) -> Self {
        self.lock_manager = Some(manager);
        self
    }

    pub fn with_function_invoke_sync(mut self, cb: FunctionInvokeSyncCallback) -> Self {
        self.function_invoke_sync = Some(cb);
        self
    }

    /// Set the default actor for job registration
    pub fn with_default_actor(mut self, actor: impl Into<String>) -> Self {
        self.default_actor = actor.into();
        self
    }

    /// Set the repository configuration for locale-aware queries.
    ///
    /// This also adopts the repository's own default language. The two are not
    /// independent: `resolve_node_for_locale` skips translation entirely when the
    /// queried locale equals `default_language`, and `get_locales_to_use` falls
    /// back to it when a query names no locale. Left at the `"en"` default while
    /// a repository declared something else, a `WHERE locale = '<repo default>'`
    /// read went looking for an overlay that by definition does not exist, and a
    /// locale-less read resolved against the wrong base — silently, since a
    /// missing overlay is not an error.
    pub fn with_repository_config(mut self, config: RepositoryConfig) -> Self {
        self.default_language = config.default_language.clone();
        self.repository_config = Some(config);
        self
    }

    /// Set the authentication context for RLS filtering
    pub fn with_auth(mut self, auth: AuthContext) -> Self {
        self.auth_context = Some(auth);
        self
    }

    /// Get the current auth context (if set)
    pub fn auth_context(&self) -> Option<&AuthContext> {
        self.auth_context.as_ref()
    }

    pub fn with_embedding_config_store(
        mut self,
        store: Arc<dyn raisin_embeddings::TenantEmbeddingConfigStore>,
    ) -> Self {
        self.embedding_config_store = Some(store);
        self
    }

    /// The tenant's configured vector-distance cutoff, read fresh per statement.
    ///
    /// `None` when no embedding config store is wired up or the tenant never set
    /// one; the search path then falls back to
    /// `raisin_hnsw::DEFAULT_MAX_DISTANCE`.
    ///
    /// Read here rather than cached on the engine so that
    /// `ALTER EMBEDDING CONFIG SET DEFAULT_MAX_DISTANCE` takes effect on the next
    /// statement instead of the next restart. A read failure is deliberately not
    /// fatal: a search at the engine default beats a query that will not run.
    pub(crate) fn tenant_default_max_distance(&self) -> Option<f32> {
        self.tenant_embedding_config()
            .ok()
            .flatten()
            .and_then(|config| config.default_max_distance)
    }

    /// The tenant's embedding config, for READING: the store wired into this
    /// engine, else the process-wide read-only reader (see
    /// [`raisin_embeddings::TenantEmbeddingConfigReader`]). Only the wired
    /// store can be written, so `ALTER EMBEDDING CONFIG` never goes through
    /// here. `Ok(None)` when the tenant has no config or no source exists.
    pub(crate) fn tenant_embedding_config(
        &self,
    ) -> Result<Option<raisin_embeddings::TenantEmbeddingConfig>, raisin_embeddings::StorageError>
    {
        if let Some(store) = self.embedding_config_store.as_ref() {
            return store.get_config(&self.tenant_id);
        }
        match raisin_embeddings::embedding_config_reader() {
            Some(reader) => reader.get_config(&self.tenant_id),
            None => Ok(None),
        }
    }

    /// Wire the tenant AI config store, so `ai_provider_ref` resolves here the
    /// same way it does on the write path.
    pub fn with_ai_config_store(
        mut self,
        store: Arc<dyn raisin_embeddings::resolve::TenantAIConfigStore>,
    ) -> Self {
        self.ai_config_store = Some(store);
        self
    }

    pub fn with_master_key(mut self, key: [u8; 32]) -> Self {
        self.master_key = Some(key);
        self
    }

    /// Set the shared schema stats cache for data-driven selectivity estimation
    pub fn with_schema_stats_cache(mut self, cache: SharedSchemaStatsCache) -> Self {
        self.schema_stats_cache = Some(cache);
        self
    }

    // =========================================================================
    // Schema Stats Loading
    // =========================================================================

    /// Schema statistics for the physical planner, from the cache (if
    /// configured).
    ///
    /// `selection` is the query's WHERE clause. The stats are used ONLY to refine
    /// selectivity for `node_type =` / `archetype =` equality (see
    /// `helpers::selection_uses_schema_stats`), so a query without one of those
    /// predicates skips this entirely: on a cache miss the computation below
    /// deserializes every NodeType and Archetype on the branch just to count
    /// them, which was ~33% of production CPU when run per query (2026-08).
    ///
    /// The gate lives HERE, not at the call sites, deliberately. Each site
    /// would otherwise need its own copy of the predicate walk, and this codebase
    /// has a documented recurring bug class of mirrored paths silently drifting —
    /// a caller that forgot the gate would quietly reintroduce the whole cost.
    /// `None` means "no WHERE clause", which cannot contain the predicate, so it
    /// skips too.
    pub(crate) async fn schema_stats_for(
        &self,
        branch: &str,
        selection: Option<&raisin_sql::analyzer::TypedExpr>,
    ) -> Option<crate::SchemaStats> {
        let uses_stats = selection.is_some_and(helpers::selection_uses_schema_stats);
        if !uses_stats {
            return None;
        }
        let stats_cache = self.schema_stats_cache.as_ref()?;
        let scope_key = format!("{}:{}:{}", self.tenant_id, self.repo_id, branch);
        let storage_ref = self.storage.clone();
        let t = self.tenant_id.clone();
        let r = self.repo_id.clone();
        let b = branch.to_string();
        let cache_stats = stats_cache
            .get_or_compute(&scope_key, || {
                let storage_ref = storage_ref.clone();
                let t = t.clone();
                let r = r.clone();
                let b = b.clone();
                async move {
                    let scope = raisin_storage::BranchScope::new(&t, &r, &b);
                    let nt_count = storage_ref
                        .node_types()
                        .list(scope, None)
                        .await
                        .map(|v| v.len())
                        .unwrap_or(0);
                    let scope = raisin_storage::BranchScope::new(&t, &r, &b);
                    let at_count = storage_ref
                        .archetypes()
                        .list(scope, None)
                        .await
                        .map(|v| v.len())
                        .unwrap_or(0);
                    Ok(raisin_core::SchemaStats {
                        node_type_count: nt_count,
                        archetype_count: at_count,
                    })
                }
            })
            .await
            .ok()?;
        Some(crate::SchemaStats {
            node_type_count: cache_stats.node_type_count,
            archetype_count: cache_stats.archetype_count,
        })
    }

    // =========================================================================
    // Branch Context Management
    // =========================================================================

    /// Get the effective branch for the current query
    ///
    /// Priority order (highest to lowest):
    /// 1. local_branch (USE LOCAL BRANCH) - single statement
    /// 2. session_branch (USE BRANCH) - persists for connection
    /// 3. branch (default from constructor)
    pub async fn effective_branch(&self) -> String {
        let local = self.local_branch.read().await;
        if let Some(ref b) = *local {
            return b.clone();
        }
        drop(local);

        let session = self.session_branch.read().await;
        if let Some(ref b) = *session {
            return b.clone();
        }
        drop(session);

        self.branch.clone()
    }

    /// Set session-level branch (USE BRANCH / SET app.branch)
    pub async fn set_session_branch(&self, branch: String) {
        *self.pending_session_branch.write().await = Some(branch.clone());
        *self.session_branch.write().await = Some(branch);
    }

    /// Set local branch for single query (USE LOCAL BRANCH)
    pub async fn set_local_branch(&self, branch: String) {
        *self.local_branch.write().await = Some(branch);
    }

    /// Clear local branch after query execution
    pub async fn clear_local_branch(&self) {
        *self.local_branch.write().await = None;
    }

    /// Get pending session branch (set by USE BRANCH in current batch)
    pub async fn get_pending_session_branch(&self) -> Option<String> {
        self.pending_session_branch.read().await.clone()
    }

    /// Take the pending session branch (consumes it)
    pub async fn take_pending_session_branch(&self) -> Option<String> {
        self.pending_session_branch.write().await.take()
    }

    /// Set session branch from transport layer
    pub fn with_session_branch(self, branch: Option<String>) -> Self {
        *self.session_branch.blocking_write() = branch;
        self
    }

    /// Execute a SQL query and return a stream of results
    pub async fn execute(&self, sql: &str) -> Result<RowStream, Error> {
        let mut timer = phase_timing::PhaseTimer::start();
        // 1. Parse and semantic analysis — and, for a query, its optimized
        // logical plan — from the prepared-statement cache when the same text
        // was seen against the same catalog (`prepared.rs`).
        let prepared = prepared::prepare_statement(&self.catalog, sql)?;
        timer.analyzed();
        self.execute_prepared(sql, prepared, timer).await
    }

    /// Execute `sql` with `$1`, `$2`, … bound to `params`, each rendered as
    /// a SQL literal by `format` (`raisin_sql::format_param_value` is what
    /// HTTP, WS and pgwire use; the functions runtime has its own).
    ///
    /// One prepared TEMPLATE serves every execution of the same text, whatever
    /// the values (`prepared_params.rs`, plan Phase 13d); a statement whose
    /// plan depends on a value is planned from the substituted text, exactly
    /// as substituting first and calling [`Self::execute`] would.
    pub async fn execute_with_params(
        &self,
        sql: &str,
        params: &[serde_json::Value],
        format: &ParamFormat,
    ) -> Result<RowStream, Error> {
        self.execute_with_params_traced(sql, params, format)
            .await
            .map(|(stream, _)| stream)
    }

    /// [`Self::execute_with_params`], also saying how the statement was
    /// prepared — for tests and diagnostics.
    #[doc(hidden)]
    pub async fn execute_with_params_traced(
        &self,
        sql: &str,
        params: &[serde_json::Value],
        format: &ParamFormat,
    ) -> Result<(RowStream, ParamOutcome), Error> {
        let mut timer = phase_timing::PhaseTimer::start();
        let bound =
            prepared_params::prepare_with_params(&self.catalog, false, sql, params, format)?;
        timer.analyzed();
        let outcome = bound.outcome;
        let prepared = bound
            .statements
            .into_iter()
            .next()
            .ok_or_else(|| Error::Validation("No statement to execute".to_string()))?;
        let stream = self.execute_prepared(&bound.sql, prepared, timer).await?;
        Ok((stream, outcome))
    }

    /// The body of [`Self::execute`] after preparation.
    async fn execute_prepared(
        &self,
        sql: &str,
        prepared: std::sync::Arc<prepared::Prepared>,
        mut timer: phase_timing::PhaseTimer,
    ) -> Result<RowStream, Error> {
        tracing::debug!("SQL Query Engine executing: {}", sql);
        // Uncorrelated subqueries (EXISTS, scalar, ANY/ALL, INSERT…SELECT) are
        // evaluated once here and folded into literals so every path below —
        // query, EXPLAIN, DML — plans against constants. A cached statement
        // never has any.
        let bound;
        let (analyzed, cached) = if subquery_bind::statement_needs_binding(&prepared.analyzed) {
            bound = self.bind_subqueries(prepared.analyzed.clone()).await?;
            (&bound, None)
        } else {
            (&prepared.analyzed, Some(prepared.as_ref()))
        };
        timer.bound();

        // Route by statement type
        let AnalyzedStatement::Query(ref query) = analyzed else {
            return self.execute_analyzed_statement(analyzed).await;
        };

        // Set function context for system functions (RAISIN_CURRENT_USER).
        // Resolves the user node only when the SQL can actually call it.
        let branch_for_lookup = query
            .branch_override
            .clone()
            .unwrap_or_else(|| self.branch.clone());
        self.install_function_context(sql, &branch_for_lookup).await;

        if let Some(scalar) = self.execute_scalar_if_no_from(analyzed).await {
            return scalar;
        }
        // 2-4. Logical plan (cached or built), physical plan (cached or
        // planned), context.
        let (physical_plan, ctx) = self.plan_query(analyzed, cached).await?;
        timer.planned();

        // 5. Execute physical plan
        let stream = execute_plan(&physical_plan, &ctx).await?;
        Ok(timer.opened(stream, sql))
    }

    /// Assemble the execution context from whatever this engine was configured
    /// with.
    ///
    /// Extracted so the typed [`search`](Self::search) entry point cannot end up
    /// wiring a DIFFERENT context from the SQL path: an engine that forgets to
    /// attach `auth_context` on one of two paths is a silent RLS bypass, and an
    /// engine that forgets `indexing_engine` on one of them is a search that
    /// quietly runs on one leg.
    fn build_execution_context(
        &self,
        branch: String,
        workspace: String,
        max_revision: Option<raisin_hlc::HLC>,
        locales: Vec<String>,
    ) -> ExecutionContext<S> {
        let mut ctx = self.new_statement_context(branch, workspace);

        ctx.default_language = Arc::from(self.default_language.as_str());
        ctx.default_max_distance = self.tenant_default_max_distance();
        ctx = ctx.with_max_revision(max_revision);
        ctx.locales = Arc::from(locales);
        // The statement's storage view (Phase 4) opens at its first batched
        // read, not here: most statements never read in batches.

        if let Some(ref engine) = self.indexing_engine {
            ctx = ctx.with_indexing_engine(engine.clone());
        }
        if let Some(ref engine) = self.hnsw_engine {
            ctx = ctx.with_hnsw_engine(engine.clone());
        }
        if let Some(ref provider) = self.embedding_provider {
            ctx = ctx.with_embedding_provider(provider.clone());
        }
        if let Some(ref storage) = self.embedding_storage {
            ctx = ctx.with_embedding_storage(storage.clone());
        }
        if let Some(ref config) = self.repository_config {
            ctx = ctx.with_repository_config(config.clone());
        }
        if let Some(ref auth) = self.auth_context {
            ctx = ctx.with_auth_context(auth.clone());
        }
        if let Some(ref cb) = self.function_invoke {
            ctx.function_invoke = Some(cb.clone());
        }
        if let Some(ref cb) = self.function_invoke_sync {
            ctx.function_invoke_sync = Some(cb.clone());
        }
        if let Some(ref mgr) = self.lock_manager {
            ctx.lock_manager = Some(mgr.clone());
        }
        ctx
    }

    /// Run a search built programmatically, with no SQL text involved.
    ///
    /// This is the entry point for the HTTP hybrid-search endpoint and the MCP
    /// `search_nodes` tool. They used to be a second and a third implementation
    /// of leg dispatch and rank fusion, and NEITHER applied row-level security:
    /// the HTTP module contained no reference to `auth` at all and the MCP
    /// provider bound its identity parameter as `_identity`. Both now go through
    /// the same scope resolver, the same per-hit `rls_filter_search_hit`, the
    /// same over-fetch/backfill loop and the same columns as SQL.
    ///
    /// Rows come back qualified with `table_name` (e.g. `hybrid_search.path`),
    /// exactly as the table function emits them.
    pub async fn search(
        &self,
        args: crate::physical_plan::search::args::SearchArgs,
        table_name: &str,
    ) -> Result<Vec<crate::physical_plan::executor::Row>, Error> {
        use futures::StreamExt;

        let max_revision = match self
            .storage
            .branches()
            .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
            .await?
        {
            Some(branch) => Some(branch.head),
            None => Some(raisin_hlc::HLC::new(0, 0)),
        };

        // The context workspace is irrelevant to a search: every hit is fetched
        // in ITS OWN workspace (that is the half of the hit key that exists for
        // this reason), and the corpus comes from the resolved scope. Passing a
        // placeholder here is deliberate -- a real-looking value would invite
        // someone to start filtering by it.
        let ctx = self.build_execution_context(
            self.branch.clone(),
            "default".to_string(),
            max_revision,
            Vec::new(),
        );

        let mut stream = crate::physical_plan::search::emit::execute_parsed(
            args,
            // No residual: an API caller has no WHERE clause. `limit` therefore
            // means exactly "rows delivered after permissions".
            None,
            table_name.to_string(),
            &ctx,
        )
        .await?;

        let mut rows = Vec::new();
        while let Some(row) = stream.next().await {
            rows.push(row?);
        }
        Ok(rows)
    }
}
