//! Request and response types for management API endpoints.

use serde::{Deserialize, Serialize};

/// Generic API response wrapper with success/error status.
#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(error: String) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(error),
        }
    }
}

#[derive(Deserialize)]
pub struct RebuildRequest {
    pub index_type: String,
}

#[derive(Deserialize)]
pub struct BackupRequest {
    pub path: String,
}

#[derive(Deserialize)]
pub struct ScheduleIntegrityRequest {
    pub tenant: String,
    pub interval_minutes: u64,
}

#[derive(Deserialize)]
pub struct RepairRequest {
    pub issues: Vec<raisin_storage::Issue>,
}

#[derive(Deserialize)]
pub struct BatchDeleteJobsRequest {
    pub job_ids: Vec<String>,
}

#[derive(Serialize)]
pub struct BatchDeleteJobsResponse {
    pub deleted: usize,
    pub skipped: usize,
}

#[derive(Serialize)]
pub struct PurgeResponse {
    pub purged: usize,
}

#[derive(Deserialize)]
pub struct ForceFailStuckRequest {
    #[serde(default = "default_stuck_minutes")]
    pub stuck_minutes: u64,
}

fn default_stuck_minutes() -> u64 {
    10
}

#[derive(Serialize)]
pub struct ForceFailStuckResponse {
    pub failed_count: usize,
    pub job_ids: Vec<String>,
}
