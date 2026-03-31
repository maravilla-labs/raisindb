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
mod batch;
mod branch;
mod handlers;
mod helpers;
mod restore;

pub use batch::batch_requires_async;

use crate::physical_plan::eval::{set_function_context, FunctionContext};
use crate::physical_plan::executor::{execute_plan, ExecutionContext, RowStream};
use crate::physical_plan::planner::PhysicalPlanner;
use crate::physical_plan::IndexCatalog;
use raisin_context::RepositoryConfig;
use raisin_embeddings::embedding_storage::EmbeddingStorage;
use raisin_embeddings::provider::EmbeddingProvider;
use raisin_error::Error;
use raisin_hnsw::HnswIndexingEngine;
use raisin_indexer::TantivyIndexingEngine;
use raisin_models::auth::AuthContext;
use raisin_sql::analyzer::{AnalyzedStatement, Analyzer, Catalog, StaticCatalog};
use raisin_sql::logical_plan::PlanBuilder;
use raisin_sql::optimizer::Optimizer;
use raisin_storage::{BranchRepository, Storage};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

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
    /// Default actor for job registration
    pub(crate) default_actor: String,
    /// Repository configuration for locale-aware queries
    pub(crate) repository_config: Option<RepositoryConfig>,
    /// Authentication context for RLS filtering
    pub(crate) auth_context: Option<AuthContext>,
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
            catalog: Arc::new(StaticCatalog::default_nodes_schema()),
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
            default_actor: "anonymous".to_string(),
            repository_config: None,
            auth_context: None,
        }
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
    pub fn with_function_invoke_sync(mut self, cb: FunctionInvokeSyncCallback) -> Self {
        self.function_invoke_sync = Some(cb);
        self
    }

    /// Set the default actor for job registration
    pub fn with_default_actor(mut self, actor: impl Into<String>) -> Self {
        self.default_actor = actor.into();
        self
    }

    /// Set the repository configuration for locale-aware queries
    pub fn with_repository_config(mut self, config: RepositoryConfig) -> Self {
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
        tracing::info!("SQL Query Engine starting execution");
        tracing::debug!("   SQL: {}", sql);
        tracing::debug!(
            "   Context: tenant={}, repo={}, default_branch={}",
            self.tenant_id,
            self.repo_id,
            self.branch
        );

        // 1. Parse and Semantic analysis
        tracing::debug!("Phase 1: Parsing and analyzing SQL");
        let analyzer = Analyzer::with_catalog(self.catalog.clone_box());
        let analyzed = analyzer
            .analyze(sql)
            .map_err(|e| Error::Validation(format!("Analysis error: {}", e)))?;

        // Route by statement type
        match &analyzed {
            AnalyzedStatement::Explain(ref explain_stmt) => {
                return self.execute_explain(explain_stmt).await;
            }
            AnalyzedStatement::Insert(_)
            | AnalyzedStatement::Update(_)
            | AnalyzedStatement::Delete(_)
            | AnalyzedStatement::Order(_)
            | AnalyzedStatement::Move(_)
            | AnalyzedStatement::Copy(_)
            | AnalyzedStatement::Translate(_)
            | AnalyzedStatement::Relate(_)
            | AnalyzedStatement::Unrelate(_) => {
                return self.execute_dml(&analyzed).await;
            }
            AnalyzedStatement::Ddl(ref ddl_stmt) => {
                return self.execute_ddl(ddl_stmt).await;
            }
            AnalyzedStatement::Transaction(ref txn_stmt) => {
                return self.execute_transaction(txn_stmt).await;
            }
            AnalyzedStatement::Show(ref show_stmt) => {
                return self.execute_show(show_stmt).await;
            }
            AnalyzedStatement::Branch(ref branch_stmt) => {
                return self.execute_branch_statement(branch_stmt).await;
            }
            AnalyzedStatement::Restore(ref restore_stmt) => {
                return self.execute_restore(restore_stmt).await;
            }
            AnalyzedStatement::Acl(ref acl_stmt) => {
                return self.execute_acl(acl_stmt).await;
            }
            AnalyzedStatement::Query(_) => {
                // Continue with query execution below
            }
        }

        // 2. Build logical plan
        let plan_builder = PlanBuilder::new(self.catalog.as_ref());
        let logical_plan = plan_builder
            .build(&analyzed)
            .map_err(|e| Error::Validation(format!("Plan error: {}", e)))?;

        // 3. Optimize logical plan
        let optimizer = Optimizer::default();
        let optimized = optimizer.optimize(logical_plan);

        // 4. Generate physical plan
        let workspace = if let AnalyzedStatement::Query(ref q) = analyzed {
            q.from
                .first()
                .and_then(|t| t.workspace.clone())
                .unwrap_or_else(|| "default".to_string())
        } else {
            "default".to_string()
        };

        let index_catalog: Arc<dyn IndexCatalog> =
            Arc::new(crate::physical_plan::catalog::RocksDBIndexCatalog::new());

        let mut physical_planner = PhysicalPlanner::with_catalog(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            workspace.clone(),
            index_catalog,
        );

        // Load compound indexes if node_type is in WHERE clause
        if let Some(node_type_name) = helpers::extract_node_type_from_analyzed(&analyzed) {
            if let Some(indexes) = helpers::load_compound_indexes(
                &*self.storage,
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                &node_type_name,
            )
            .await
            {
                physical_planner.set_compound_indexes(indexes);
            }
        }

        let physical_plan = physical_planner.plan(&optimized)?;

        // 5. Create execution context
        let (max_revision, branch_override, locales) =
            if let AnalyzedStatement::Query(ref q) = analyzed {
                (q.max_revision, q.branch_override.clone(), q.locales.clone())
            } else {
                (None, None, Vec::new())
            };

        let branch = branch_override.unwrap_or_else(|| self.branch.clone());

        let max_revision = if max_revision.is_none() {
            let branch_opt = self
                .storage
                .branches()
                .get_branch(&self.tenant_id, &self.repo_id, &branch)
                .await?;

            Some(
                branch_opt
                    .map(|b| b.head)
                    .unwrap_or_else(|| raisin_hlc::HLC::new(0, 0)),
            )
        } else {
            max_revision
        };

        // Save branch for user lookup
        let branch_for_lookup = branch.clone();

        let mut ctx = ExecutionContext::new(
            self.storage.clone(),
            self.tenant_id.clone(),
            self.repo_id.clone(),
            branch,
            workspace,
        );

        ctx.default_language = Arc::from(self.default_language.as_str());
        ctx = ctx.with_max_revision(max_revision);
        ctx.locales = Arc::from(locales);

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

        // Set function context for system functions (CURRENT_USER)
        if let Some(ref auth) = self.auth_context {
            tracing::info!(
                "[execute] Setting up function context: user_id={:?}",
                auth.user_id
            );

            let user_node = if let Some(ref user_id) = auth.user_id {
                let node = self.lookup_user_node(user_id, &branch_for_lookup).await;
                tracing::info!(
                    "[execute] lookup_user_node result for user_id={}: found={}",
                    user_id,
                    node.is_some()
                );
                node
            } else {
                tracing::warn!("[execute] auth_context exists but user_id is None");
                None
            };

            set_function_context(FunctionContext {
                user_id: auth.user_id.clone(),
                user_node: user_node.clone(),
            });

            tracing::info!(
                "[execute] Function context set: user_id={:?}, has_node={}",
                auth.user_id,
                user_node.is_some()
            );
        } else {
            tracing::warn!("[execute] No auth_context available, using default");
            set_function_context(FunctionContext::default());
        }

        // 6. Execute physical plan
        let stream = execute_plan(&physical_plan, &ctx).await?;
        Ok(stream)
    }
}
