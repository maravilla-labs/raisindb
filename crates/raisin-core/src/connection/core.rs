//! Server-side connection and configuration types.

use raisin_storage::Storage;
use std::sync::Arc;

use super::management::RepositoryManagement;
use super::tenant::TenantScope;

/// Configuration for server connection.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Default branch for new repositories
    pub default_branch: String,

    /// Whether to auto-create missing repositories
    pub auto_create_repositories: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            default_branch: "main".to_string(),
            auto_create_repositories: false,
        }
    }
}

/// Server-side connection wrapping a storage backend.
///
/// This is the main entry point for server-side operations. Used internally by
/// raisin-server and raisin-transport-http.
///
/// # MongoDB Analogy
///
/// This is similar to MongoDB's server-side `MongoClient` with direct database access.
///
/// # Example
///
/// ```rust,no_run
/// use raisin_core::RaisinConnection;
/// use raisin_storage_memory::InMemoryStorage;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let storage = Arc::new(InMemoryStorage::default());
/// let connection = RaisinConnection::with_storage(storage);
///
/// // Always scope to a tenant first
/// let tenant = connection.tenant("acme-corp");
/// let repo = tenant.repository("website");
/// # Ok(())
/// # }
/// ```
pub struct RaisinConnection<S: Storage> {
    pub(super) storage: Arc<S>,
    pub(super) config: ServerConfig,
}

impl<S: Storage> RaisinConnection<S> {
    /// Create a new connection with an existing storage backend.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use raisin_core::RaisinConnection;
    /// use raisin_storage_memory::InMemoryStorage;
    /// use std::sync::Arc;
    ///
    /// let storage = Arc::new(InMemoryStorage::default());
    /// let connection = RaisinConnection::with_storage(storage);
    /// ```
    pub fn with_storage(storage: Arc<S>) -> Self {
        Self {
            storage,
            config: ServerConfig::default(),
        }
    }

    /// Create a connection with custom configuration.
    pub fn with_config(storage: Arc<S>, config: ServerConfig) -> Self {
        Self { storage, config }
    }

    /// Scope operations to a specific tenant.
    ///
    /// This is **always required** - there is no direct repository access.
    ///
    /// For embedded/single-tenant deployments, use a hardcoded constant:
    ///
    /// ```rust,no_run
    /// # use raisin_core::RaisinConnection;
    /// # use raisin_storage_memory::InMemoryStorage;
    /// # use std::sync::Arc;
    /// # let storage = Arc::new(InMemoryStorage::default());
    /// # let connection = RaisinConnection::with_storage(storage);
    /// const DEFAULT_TENANT: &str = "default";
    /// let tenant = connection.tenant(DEFAULT_TENANT);
    /// ```
    pub fn tenant(&self, tenant_id: impl Into<String>) -> TenantScope<S> {
        TenantScope {
            connection: self,
            tenant_id: tenant_id.into(),
        }
    }

    /// Access repository management operations (admin/system level).
    ///
    /// This provides cross-tenant repository management for admin operations.
    pub fn repository_management(&self) -> RepositoryManagement<S> {
        RepositoryManagement { connection: self }
    }

    /// Get underlying storage (for advanced use).
    pub fn storage(&self) -> &Arc<S> {
        &self.storage
    }

    /// Get server configuration.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }
}

/// Internal wrapper to hold `Arc<Storage>` and config for owned references.
pub(super) struct RaisinConnectionArc<S: Storage>(pub(super) Arc<S>, pub(super) ServerConfig);
