//! Workspace configuration and scope types.

use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::RepositoryContext;

/// Workspace configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Workspace identifier
    pub workspace_id: String,

    /// Default branch for this workspace
    pub default_branch: String,

    /// NodeType version pinning: type name -> HLC revision (None = track latest)
    #[serde(default, alias = "node_type_refs")]
    pub node_type_pins: std::collections::HashMap<String, Option<HLC>>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            workspace_id: "main".to_string(),
            default_branch: "main".to_string(),
            node_type_pins: std::collections::HashMap::new(),
        }
    }
}

/// Workspace scope for operations
///
/// Lightweight struct passed to storage/service APIs when operating on a specific workspace.
#[derive(Debug, Clone)]
pub struct WorkspaceScope {
    /// Repository context
    pub repository: Arc<RepositoryContext>,

    /// Workspace identifier
    pub workspace_id: String,

    /// Optional branch (None = use workspace default)
    pub branch: Option<String>,

    /// Optional revision for time-travel reads
    pub as_of_revision: Option<u64>,
}

impl WorkspaceScope {
    /// Create a new workspace scope
    pub fn new(repository: Arc<RepositoryContext>, workspace_id: impl Into<String>) -> Self {
        Self {
            repository,
            workspace_id: workspace_id.into(),
            branch: None,
            as_of_revision: None,
        }
    }

    /// Set the branch
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Set the revision
    pub fn with_revision(mut self, revision: u64) -> Self {
        self.as_of_revision = Some(revision);
        self
    }

    /// Get the effective branch (or default if None)
    pub fn effective_branch<'a>(&'a self, default: &'a str) -> &'a str {
        self.branch.as_deref().unwrap_or(default)
    }
}
