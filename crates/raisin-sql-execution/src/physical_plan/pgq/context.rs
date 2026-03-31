//! PGQ Execution Context
//!
//! Contains all context needed for graph query execution.

use raisin_hlc::HLC;

/// Execution context for PGQ queries
///
/// Contains tenant, repository, branch, and workspace information
/// needed to execute graph queries against the storage layer.
#[derive(Debug, Clone)]
pub struct PgqContext {
    /// Workspace identifier (for scoping queries)
    pub workspace_id: String,
    /// Tenant identifier
    pub tenant_id: String,
    /// Repository identifier
    pub repo_id: String,
    /// Branch name
    pub branch: String,
    /// Optional revision for point-in-time queries
    pub revision: Option<HLC>,
}

impl PgqContext {
    /// Create a new PGQ execution context
    pub fn new(
        workspace_id: String,
        tenant_id: String,
        repo_id: String,
        branch: String,
        revision: Option<HLC>,
    ) -> Self {
        Self {
            workspace_id,
            tenant_id,
            repo_id,
            branch,
            revision,
        }
    }
}
