//! Request and response types for system updates endpoints.

use raisin_storage::system_updates::{PendingUpdatesSummary, ResourceType};
use serde::{Deserialize, Serialize};

/// Response for pending updates check
#[derive(Debug, Serialize)]
pub struct PendingUpdatesResponse {
    /// Whether there are any pending updates
    pub has_updates: bool,
    /// Total number of pending updates
    pub total_pending: usize,
    /// Number of updates with breaking changes
    pub breaking_count: usize,
    /// List of pending updates
    pub updates: Vec<PendingUpdateInfo>,
}

/// Information about a single pending update
#[derive(Debug, Serialize)]
pub struct PendingUpdateInfo {
    /// Type of resource (NodeType or Workspace)
    pub resource_type: String,
    /// Name of the resource
    pub name: String,
    /// Whether this is a new resource (never applied)
    pub is_new: bool,
    /// Whether this update contains breaking changes
    pub is_breaking: bool,
    /// Number of breaking changes
    pub breaking_count: usize,
    /// New version (if available)
    pub new_version: Option<i32>,
    /// Currently applied version (if available)
    pub old_version: Option<i32>,
}

impl From<PendingUpdatesSummary> for PendingUpdatesResponse {
    fn from(summary: PendingUpdatesSummary) -> Self {
        PendingUpdatesResponse {
            has_updates: summary.has_updates,
            total_pending: summary.total_pending,
            breaking_count: summary.breaking_count,
            updates: summary
                .updates
                .into_iter()
                .map(|u| PendingUpdateInfo {
                    resource_type: match u.resource_type {
                        ResourceType::NodeType => "NodeType".to_string(),
                        ResourceType::Workspace => "Workspace".to_string(),
                        ResourceType::Package => "Package".to_string(),
                    },
                    name: u.name,
                    is_new: u.old_hash.is_none(),
                    is_breaking: u.is_breaking,
                    breaking_count: u.breaking_changes.len(),
                    new_version: u.new_version,
                    old_version: u.old_version,
                })
                .collect(),
        }
    }
}

/// Request to apply system updates
#[derive(Debug, Deserialize)]
pub struct ApplyUpdatesRequest {
    /// Specific resources to update (empty = all pending)
    #[serde(default)]
    pub resources: Vec<String>,
    /// Force apply even with breaking changes
    #[serde(default)]
    pub force: bool,
}

/// Response for apply updates request
#[derive(Debug, Serialize)]
pub struct ApplyUpdatesResponse {
    /// Job ID for tracking the update (if async)
    pub job_id: Option<String>,
    /// Message describing the result
    pub message: String,
    /// Number of updates applied
    pub applied_count: usize,
    /// Number of updates skipped (due to breaking changes)
    pub skipped_count: usize,
}
