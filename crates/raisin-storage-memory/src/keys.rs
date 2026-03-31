//! Key generation helpers for in-memory storage
//!
//! This module provides consistent key formatting for all in-memory storage operations.
//! The key structure follows the repository-first architecture:
//!
//! ```text
//! /{tenant_id}/repo/{repo_id}/branch/{branch}/workspace/{workspace}/nodes/{node_id}
//! ```

/// Node key components for in-memory storage
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeKey {
    /// Tenant identifier
    pub tenant_id: String,

    /// Repository identifier
    pub repo_id: String,

    /// Branch name
    pub branch: String,

    /// Workspace identifier
    pub workspace: String,

    /// Node identifier
    pub node_id: String,
}

impl NodeKey {
    /// Create a new node key
    pub fn new(
        tenant_id: impl Into<String>,
        repo_id: impl Into<String>,
        branch: impl Into<String>,
        workspace: impl Into<String>,
        node_id: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            branch: branch.into(),
            workspace: workspace.into(),
            node_id: node_id.into(),
        }
    }

    /// Generate the full key path
    ///
    /// Format: `/{tenant_id}/repo/{repo_id}/branch/{branch}/workspace/{workspace}/nodes/{node_id}`
    pub fn to_path(&self) -> String {
        format!(
            "/{}/repo/{}/branch/{}/workspace/{}/nodes/{}",
            self.tenant_id, self.repo_id, self.branch, self.workspace, self.node_id
        )
    }

    /// Create a prefix for listing all nodes in a workspace
    ///
    /// Format: `/{tenant_id}/repo/{repo_id}/branch/{branch}/workspace/{workspace}/nodes/`
    pub fn workspace_prefix(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> String {
        format!(
            "/{}/repo/{}/branch/{}/workspace/{}/nodes/",
            tenant_id, repo_id, branch, workspace
        )
    }

    /// Create a prefix for all nodes in a repository
    pub fn repository_prefix(tenant_id: &str, repo_id: &str) -> String {
        format!("/{}/repo/{}/", tenant_id, repo_id)
    }

    /// Create a prefix for all nodes in a branch
    pub fn branch_prefix(tenant_id: &str, repo_id: &str, branch: &str) -> String {
        format!("/{}/repo/{}/branch/{}/", tenant_id, repo_id, branch)
    }
}

/// Workspace key for workspace metadata
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceKey {
    pub tenant_id: String,
    pub repo_id: String,
    pub workspace_id: String,
}

impl WorkspaceKey {
    pub fn new(
        tenant_id: impl Into<String>,
        repo_id: impl Into<String>,
        workspace_id: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            workspace_id: workspace_id.into(),
        }
    }

    pub fn to_path(&self) -> String {
        format!(
            "/{}/repo/{}/workspace/{}",
            self.tenant_id, self.repo_id, self.workspace_id
        )
    }
}

/// NodeType key (global per tenant)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeTypeKey {
    pub tenant_id: String,
    pub name: String,
}

impl NodeTypeKey {
    pub fn new(tenant_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            name: name.into(),
        }
    }

    pub fn to_path(&self) -> String {
        format!("/{}/nodetypes/{}", self.tenant_id, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_key_path() {
        let key = NodeKey::new("tenant-1", "website", "main", "content", "article-123");
        assert_eq!(
            key.to_path(),
            "/tenant-1/repo/website/branch/main/workspace/content/nodes/article-123"
        );
    }

    #[test]
    fn test_workspace_prefix() {
        let prefix = NodeKey::workspace_prefix("tenant-1", "website", "main", "content");
        assert_eq!(
            prefix,
            "/tenant-1/repo/website/branch/main/workspace/content/nodes/"
        );
    }

    #[test]
    fn test_repository_prefix() {
        let prefix = NodeKey::repository_prefix("tenant-1", "website");
        assert_eq!(prefix, "/tenant-1/repo/website/");
    }

    #[test]
    fn test_branch_prefix() {
        let prefix = NodeKey::branch_prefix("tenant-1", "website", "develop");
        assert_eq!(prefix, "/tenant-1/repo/website/branch/develop/");
    }

    #[test]
    fn test_workspace_key() {
        let key = WorkspaceKey::new("tenant-1", "website", "main");
        assert_eq!(key.to_path(), "/tenant-1/repo/website/workspace/main");
    }

    #[test]
    fn test_nodetype_key() {
        let key = NodeTypeKey::new("tenant-1", "raisin:Page");
        assert_eq!(key.to_path(), "/tenant-1/nodetypes/raisin:Page");
    }
}
