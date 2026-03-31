//! Repository context for repository-first architecture.

use serde::{Deserialize, Serialize};

/// Internal context for repository operations.
/// Always includes tenant_id (never optional).
///
/// This is the primary scoping mechanism for the repository-first architecture.
/// It replaces the old TenantContext approach with explicit repository isolation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RepositoryContext {
    /// Tenant ID (always present, use "default" for embedded mode)
    tenant_id: String,

    /// Repository identifier (e.g., "website", "mobile-app")
    repository_id: String,

    /// Storage key prefix: "/{tenant_id}/repo/{repo_id}"
    storage_prefix: String,
}

impl RepositoryContext {
    /// Create a new repository context
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_context::RepositoryContext;
    ///
    /// // Multi-tenant: explicit tenant
    /// let ctx = RepositoryContext::new("acme-corp", "website");
    /// assert_eq!(ctx.tenant_id(), "acme-corp");
    /// assert_eq!(ctx.repository_id(), "website");
    /// assert_eq!(ctx.storage_prefix(), "/acme-corp/repo/website");
    ///
    /// // Single-tenant: hardcoded "default" tenant
    /// let ctx = RepositoryContext::new("default", "my-app");
    /// assert_eq!(ctx.tenant_id(), "default");
    /// ```
    pub fn new(tenant_id: impl Into<String>, repository_id: impl Into<String>) -> Self {
        let tenant_id = tenant_id.into();
        let repository_id = repository_id.into();
        let storage_prefix = format!("/{}/repo/{}", tenant_id, repository_id);

        Self {
            tenant_id,
            repository_id,
            storage_prefix,
        }
    }

    /// Get the tenant ID
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// Get the repository ID
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    /// Get the storage prefix
    ///
    /// Format: `/{tenant_id}/repo/{repo_id}`
    pub fn storage_prefix(&self) -> &str {
        &self.storage_prefix
    }

    /// Generate a workspace storage prefix
    ///
    /// Format: `/{tenant_id}/repo/{repo_id}/workspace/{workspace_id}`
    pub fn workspace_prefix(&self, workspace_id: &str) -> String {
        format!("{}/workspace/{}", self.storage_prefix, workspace_id)
    }

    /// Generate a branch storage prefix
    ///
    /// Format: `/{tenant_id}/repo/{repo_id}/branch/{branch}`
    pub fn branch_prefix(&self, branch: &str) -> String {
        format!("{}/branch/{}", self.storage_prefix, branch)
    }

    /// Generate a full node key with branch and workspace
    ///
    /// Format: `/{tenant_id}/repo/{repo_id}/branch/{branch}/workspace/{workspace}/nodes/{node_id}`
    pub fn node_key(&self, branch: &str, workspace: &str, node_id: &str) -> String {
        format!(
            "{}/branch/{}/workspace/{}/nodes/{}",
            self.storage_prefix, branch, workspace, node_id
        )
    }
}
