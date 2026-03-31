//! Repository handle within a tenant scope.

use raisin_context::RepositoryContext;
use raisin_storage::Storage;
use std::sync::Arc;

use super::core::{RaisinConnectionArc, ServerConfig};
use super::workspace::Workspace;

/// Handle to a specific repository (database).
///
/// Provides access to workspaces, branches, and management operations within
/// a repository.
///
/// # MongoDB Analogy
///
/// This is similar to MongoDB's `Database` handle.
///
/// # Example
///
/// ```rust,no_run
/// # use raisin_core::RaisinConnection;
/// # use raisin_storage_memory::InMemoryStorage;
/// # use std::sync::Arc;
/// # let storage = Arc::new(InMemoryStorage::default());
/// # let connection = RaisinConnection::with_storage(storage);
/// # let tenant = connection.tenant("acme-corp");
/// let repo = tenant.repository("website");
/// let workspace = repo.workspace("main");
/// let nodes = workspace.nodes();
/// ```
pub struct Repository<S: Storage> {
    pub(super) connection: Arc<RaisinConnectionArc<S>>,
    pub(super) tenant_id: String,
    pub(super) repo_id: String,
    pub(super) context: Arc<RepositoryContext>,
}

impl<S: Storage> Repository<S> {
    /// Access a workspace within this repository.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use raisin_core::RaisinConnection;
    /// # use raisin_storage_memory::InMemoryStorage;
    /// # use std::sync::Arc;
    /// # let storage = Arc::new(InMemoryStorage::default());
    /// # let connection = RaisinConnection::with_storage(storage);
    /// # let tenant = connection.tenant("acme-corp");
    /// # let repo = tenant.repository("website");
    /// let main_workspace = repo.workspace("main");
    /// let blog_workspace = repo.workspace("blog");
    /// ```
    pub fn workspace(&self, workspace_id: impl Into<String>) -> Workspace<S> {
        Workspace {
            repository: self,
            workspace_id: workspace_id.into(),
        }
    }

    /// Get the repository context.
    pub fn context(&self) -> &Arc<RepositoryContext> {
        &self.context
    }

    /// Get the tenant ID.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// Get the repository ID.
    pub fn repo_id(&self) -> &str {
        &self.repo_id
    }

    /// Access underlying storage.
    pub(crate) fn storage(&self) -> &Arc<S> {
        &self.connection.0
    }

    /// Get server configuration.
    pub(crate) fn config(&self) -> &ServerConfig {
        &self.connection.1
    }
}
