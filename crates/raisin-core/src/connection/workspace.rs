//! Workspace handle and node service builder.

use raisin_hlc::HLC;
use raisin_storage::Storage;

use super::repository::Repository;

/// Handle to a workspace within a repository.
///
/// Operations default to HEAD of the workspace's default branch.
///
/// # MongoDB Analogy
///
/// This is similar to MongoDB's `Collection` handle, but with additional
/// branch and revision context.
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
/// let workspace = repo.workspace("main");
/// let nodes = workspace.nodes();
/// let develop_nodes = workspace.nodes().branch("develop");
/// ```
pub struct Workspace<'r, S: Storage> {
    pub(super) repository: &'r Repository<S>,
    pub(super) workspace_id: String,
}

impl<'r, S: Storage> Workspace<'r, S> {
    /// Get node service builder for operations.
    ///
    /// Returns a builder that can be refined with `.branch()` or `.revision()`
    /// before performing operations.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// # use raisin_core::RaisinConnection;
    /// # use raisin_storage_memory::InMemoryStorage;
    /// # use std::sync::Arc;
    /// # let storage = Arc::new(InMemoryStorage::default());
    /// # let connection = RaisinConnection::with_storage(storage);
    /// # let tenant = connection.tenant("acme-corp");
    /// # let repo = tenant.repository("website");
    /// # let workspace = repo.workspace("main");
    /// // Default: HEAD of default branch
    /// let nodes = workspace.nodes();
    ///
    /// // Specific branch
    /// let develop = workspace.nodes().branch("develop");
    ///
    /// // Time-travel to specific revision
    /// let historical = workspace.nodes().revision(100);
    /// ```
    pub fn nodes(&'r self) -> NodeServiceBuilder<'r, S> {
        NodeServiceBuilder {
            workspace: self,
            branch: None,
            revision: None,
        }
    }

    /// Get the workspace ID.
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    /// Get the repository handle.
    pub fn repository(&self) -> &Repository<S> {
        self.repository
    }
}

/// Builder for NodeService with context refinement.
///
/// Start with defaults, refine with `.branch()` or `.revision()`.
///
/// # Example
///
/// ```rust,ignore
/// # use raisin_core::RaisinConnection;
/// # use raisin_storage_memory::InMemoryStorage;
/// # use std::sync::Arc;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let storage = Arc::new(InMemoryStorage::default());
/// # let connection = RaisinConnection::with_storage(storage);
/// # let tenant = connection.tenant("acme-corp");
/// # let repo = tenant.repository("website");
/// # let workspace = repo.workspace("main");
/// // Default context
/// let nodes = workspace.nodes();
///
/// // Refine to specific branch
/// let develop = workspace.nodes().branch("develop");
///
/// // Time-travel to revision
/// let v42 = workspace.nodes().revision(42);
///
/// // Perform operations
/// let node = nodes.get("node-id").await?;
/// # Ok(())
/// # }
/// ```
pub struct NodeServiceBuilder<'w, S: Storage> {
    pub(super) workspace: &'w Workspace<'w, S>,
    pub(super) branch: Option<String>,
    pub(super) revision: Option<HLC>,
}

impl<'w, S: Storage> NodeServiceBuilder<'w, S> {
    /// Set specific branch (overrides workspace default).
    pub fn branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Set specific revision for time-travel reads (point-in-time snapshot).
    pub fn revision(mut self, rev: HLC) -> Self {
        self.revision = Some(rev);
        self
    }

    /// Get the effective branch (or default).
    pub(crate) fn effective_branch(&self) -> &str {
        self.branch.as_deref().unwrap_or("main") // TODO: Get from workspace config
    }

    /// Get the workspace.
    pub(crate) fn workspace(&self) -> &Workspace<'w, S> {
        self.workspace
    }

    /// Get the repository.
    pub(crate) fn repository(&self) -> &Repository<S> {
        self.workspace.repository()
    }
}
