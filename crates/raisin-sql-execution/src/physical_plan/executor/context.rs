//! Execution context for physical plan execution.
//!
//! Provides the `ExecutionContext` struct that carries storage references,
//! indexing engines, and query-scoped configuration through the operator tree.

use super::row::CachedEmbedding;
use crate::physical_plan::cte_storage::{CTEConfig, MaterializedCTE};
use raisin_context::RepositoryConfig;
use raisin_embeddings::provider::EmbeddingProvider;
use raisin_embeddings::EmbeddingStorage;
use raisin_hnsw::HnswIndexingEngine;
use raisin_indexer::TantivyIndexingEngine;
use raisin_models::auth::AuthContext;
use raisin_models::translations::LocaleCode;
use raisin_storage::Storage;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Execution context containing storage, indexes, and query parameters
///
/// This context is passed to all operators during execution and provides
/// access to the storage layer and indexing engines.
pub struct ExecutionContext<S: Storage> {
    /// Storage implementation (RocksDB)
    pub storage: Arc<S>,
    /// Full-text search engine (Tantivy)
    pub indexing_engine: Option<Arc<TantivyIndexingEngine>>,
    /// Vector similarity search engine (HNSW)
    pub hnsw_engine: Option<Arc<HnswIndexingEngine>>,
    /// Embedding provider for EMBEDDING() function evaluation
    pub embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    /// Embedding storage for fetching node embeddings from RocksDB
    /// Used to populate the virtual `embedding` column in query results
    pub embedding_storage: Option<Arc<dyn EmbeddingStorage>>,
    /// Cache for EMBEDDING() function results to avoid redundant API calls
    /// Maps input text to cached embedding with TTL (default: 60 seconds)
    pub embedding_cache: Arc<RwLock<HashMap<String, CachedEmbedding>>>,
    /// TTL for embedding cache entries (default: 60 seconds)
    pub embedding_cache_ttl: Duration,
    /// Tenant identifier (Arc for cheap cloning in async streams)
    pub tenant_id: Arc<str>,
    /// Repository identifier (Arc for cheap cloning in async streams)
    pub repo_id: Arc<str>,
    /// Branch name (Arc for cheap cloning in async streams)
    pub branch: Arc<str>,
    /// Workspace identifier (Arc for cheap cloning in async streams)
    pub workspace: Arc<str>,
    /// Maximum revision to read (for point-in-time queries)
    pub max_revision: Option<raisin_hlc::HLC>,
    /// Locale for translation resolution
    ///
    /// When set, query results will be translated to this locale using
    /// the repository's configured fallback chain. If None, nodes are
    /// returned in their base language.
    pub locale: Option<LocaleCode>,
    /// Locales extracted from WHERE clause (e.g., WHERE locale = 'en' or WHERE locale IN ('en', 'de'))
    /// Empty vec = no locale filtering, use default behavior
    /// Non-empty vec = use these locales for translation resolution, return one row per locale per node
    /// (Arc for cheap cloning in async streams)
    pub locales: Arc<[String]>,
    /// Default language from repository configuration
    /// Used when no locale is specified in the query (fallback for empty locales vec)
    /// (Arc for cheap cloning in async streams)
    pub default_language: Arc<str>,
    /// Storage for materialized Common Table Expressions (CTEs)
    ///
    /// Maps CTE names to their materialized result sets, which may be
    /// stored in memory or spilled to disk depending on size.
    /// Uses RwLock for interior mutability during CTE materialization.
    pub cte_storage: Arc<RwLock<HashMap<String, MaterializedCTE>>>,
    /// Configuration for CTE materialization and spillage
    ///
    /// Controls memory limits and temporary file locations for CTE spillage.
    pub cte_config: CTEConfig,
    /// Temporary files created during query execution
    ///
    /// Tracks temp files for automatic cleanup when the context is dropped.
    /// This includes CTE spill files and other temporary storage.
    /// Uses RwLock for interior mutability during execution.
    pub temp_files: Arc<RwLock<Vec<PathBuf>>>,
    /// Active transaction context for DML operations
    ///
    /// When set, DML operations (INSERT, UPDATE, DELETE) will use this context
    /// instead of creating auto-commit transactions. Shared across operations
    /// within a transaction using Arc<RwLock>.
    pub transaction_context:
        Arc<RwLock<Option<Box<dyn raisin_storage::transactional::TransactionalContext>>>>,
    /// Repository configuration for locale-aware queries
    ///
    /// When set along with locales, scan executors will use the TranslationResolver
    /// to translate nodes according to the repository's locale fallback chains.
    /// This enables SQL queries like `WHERE locale = 'fr'` to return French translations.
    pub repository_config: Option<RepositoryConfig>,
    /// Authentication context for Row-Level Security (RLS) filtering
    ///
    /// When set, query results will be filtered based on the user's permissions.
    /// System context bypasses all RLS checks.
    pub auth_context: Option<AuthContext>,
    /// Optional callback for async function invocation (INVOKE)
    pub function_invoke: Option<crate::engine::FunctionInvokeCallback>,
    /// Optional callback for sync function invocation (INVOKE_SYNC)
    pub function_invoke_sync: Option<crate::engine::FunctionInvokeSyncCallback>,
}

impl<S: Storage> ExecutionContext<S> {
    /// Create a new execution context
    pub fn new(
        storage: Arc<S>,
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace: String,
    ) -> Self {
        Self {
            storage,
            indexing_engine: None,
            hnsw_engine: None,
            embedding_provider: None,
            embedding_storage: None,
            embedding_cache: Arc::new(RwLock::new(HashMap::new())),
            embedding_cache_ttl: Duration::from_secs(60), // Default 60 seconds
            tenant_id: Arc::from(tenant_id),
            repo_id: Arc::from(repo_id),
            branch: Arc::from(branch),
            workspace: Arc::from(workspace),
            max_revision: None,
            locale: None,                   // Default: no translation, use base language
            locales: Arc::from(Vec::new()), // Default: no locale filtering
            default_language: Arc::from("en"), // Default fallback, should be set by QueryEngine
            cte_storage: Arc::new(RwLock::new(HashMap::new())),
            cte_config: CTEConfig::default(),
            temp_files: Arc::new(RwLock::new(Vec::new())),
            transaction_context: Arc::new(RwLock::new(None)),
            repository_config: None, // Default: no translation, must be set for locale support
            auth_context: None,      // Default: no RLS filtering (system context)
            function_invoke: None,
            function_invoke_sync: None,
        }
    }

    /// Set the authentication context for RLS filtering
    pub fn with_auth_context(mut self, auth: AuthContext) -> Self {
        self.auth_context = Some(auth);
        self
    }

    /// Set the full-text indexing engine
    pub fn with_indexing_engine(mut self, engine: Arc<TantivyIndexingEngine>) -> Self {
        self.indexing_engine = Some(engine);
        self
    }

    /// Set the HNSW vector search engine
    pub fn with_hnsw_engine(mut self, engine: Arc<HnswIndexingEngine>) -> Self {
        self.hnsw_engine = Some(engine);
        self
    }

    /// Set the embedding provider for EMBEDDING() function evaluation
    pub fn with_embedding_provider(mut self, provider: Arc<dyn EmbeddingProvider>) -> Self {
        self.embedding_provider = Some(provider);
        self
    }

    /// Set the embedding storage for populating virtual `embedding` column
    ///
    /// This allows SQL queries to access node embeddings stored in RocksDB.
    /// When the `embedding` column is selected, it will be populated from
    /// the embedding storage using the node's ID and revision.
    pub fn with_embedding_storage(mut self, storage: Arc<dyn EmbeddingStorage>) -> Self {
        self.embedding_storage = Some(storage);
        self
    }

    /// Set the maximum revision for point-in-time queries
    /// None = HEAD (latest), Some(rev) = specific revision
    pub fn with_max_revision(mut self, max_revision: Option<raisin_hlc::HLC>) -> Self {
        self.max_revision = max_revision;
        self
    }

    /// Set the locale for translation resolution
    ///
    /// When a locale is set, query results will be translated using the
    /// repository's configured fallback chain. If the node is hidden in
    /// the specified locale (via LocaleOverlay::Hidden), it will be
    /// excluded from results.
    ///
    /// # Arguments
    ///
    /// * `locale` - Optional locale code (e.g., "fr", "de", "es-MX")
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use raisin_models::translations::LocaleCode;
    ///
    /// let ctx = ExecutionContext::new(storage, tenant, repo, branch, workspace)
    ///     .with_locale(Some(LocaleCode::parse("fr")?));
    /// ```
    pub fn with_locale(mut self, locale: Option<LocaleCode>) -> Self {
        self.locale = locale;
        self
    }

    /// Set the branch for cross-branch queries
    ///
    /// This overrides the default branch specified in the QueryEngine constructor.
    /// None uses the default branch, Some(name) queries a specific branch.
    pub fn with_branch(mut self, branch: String) -> Self {
        self.branch = Arc::from(branch);
        self
    }

    /// Set a custom CTE configuration
    ///
    /// This allows customizing memory limits and temporary file locations
    /// for Common Table Expression materialization and spillage.
    ///
    /// # Arguments
    ///
    /// * `config` - CTE configuration with memory limits and temp dir settings
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use raisin_sql::physical_plan::cte_storage::CTEConfig;
    ///
    /// let config = CTEConfig::new(50 * 1024 * 1024); // 50MB memory limit
    /// let ctx = ExecutionContext::new(storage, tenant, repo, branch, workspace)
    ///     .with_cte_config(config);
    /// ```
    pub fn with_cte_config(mut self, config: CTEConfig) -> Self {
        self.cte_config = config;
        self
    }

    /// Set the repository configuration for locale-aware queries
    ///
    /// When set, scan executors will use the TranslationResolver to translate
    /// nodes according to the repository's locale fallback chains. This enables
    /// SQL queries like `WHERE locale = 'fr'` to return French translations.
    ///
    /// # Arguments
    ///
    /// * `config` - Repository configuration with locale fallback chains
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Fetch repository config (contains locale fallback chains)
    /// let repository = storage.repository_management()
    ///     .get_repository(tenant_id, repo_id).await?.unwrap();
    ///
    /// let ctx = ExecutionContext::new(storage, tenant, repo, branch, workspace)
    ///     .with_repository_config(repository.config);
    /// ```
    pub fn with_repository_config(mut self, config: RepositoryConfig) -> Self {
        self.repository_config = Some(config);
        self
    }
}

impl<S: Storage> Clone for ExecutionContext<S> {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            indexing_engine: self.indexing_engine.clone(),
            hnsw_engine: self.hnsw_engine.clone(),
            embedding_provider: self.embedding_provider.clone(),
            embedding_storage: self.embedding_storage.clone(),
            embedding_cache: self.embedding_cache.clone(), // Shared cache across clones
            embedding_cache_ttl: self.embedding_cache_ttl, // Shared TTL setting
            tenant_id: self.tenant_id.clone(),
            repo_id: self.repo_id.clone(),
            branch: self.branch.clone(),
            workspace: self.workspace.clone(),
            max_revision: self.max_revision,
            locale: self.locale.clone(), // Clone locale for translation resolution
            locales: self.locales.clone(), // Clone locales for multi-locale support
            default_language: self.default_language.clone(), // Clone default language
            cte_storage: Arc::new(RwLock::new(HashMap::new())), // Note: CTEs are not cloned, each clone gets empty storage
            cte_config: self.cte_config.clone(),
            temp_files: Arc::new(RwLock::new(Vec::new())), // Note: temp_files are not shared across clones
            transaction_context: self.transaction_context.clone(), // Shared transaction context across clones
            repository_config: self.repository_config.clone(), // Clone repository config for translation resolution
            auth_context: self.auth_context.clone(), // Clone auth context for RLS filtering
            function_invoke: self.function_invoke.clone(),
            function_invoke_sync: self.function_invoke_sync.clone(),
        }
    }
}

impl<S: Storage> Drop for ExecutionContext<S> {
    fn drop(&mut self) {
        // Clean up all temporary files
        // Use try_read() for non-blocking access in Drop (can't be async)
        if let Ok(temp_files) = self.temp_files.try_read() {
            for path in temp_files.iter() {
                if let Err(e) = std::fs::remove_file(path) {
                    tracing::warn!("Failed to delete CTE temp file {:?}: {}", path, e);
                } else {
                    tracing::debug!("Cleaned up temp file: {:?}", path);
                }
            }
        }
    }
}
