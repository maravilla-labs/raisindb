//! HTTP Management API endpoints for RaisinDB.
//!
//! Provides HTTP endpoints for management operations including:
//! - Health checks
//! - Integrity scanning
//! - Index rebuilding and verification
//! - Backup/restore
//! - Compaction and metrics
//! - Background job management

mod backup;
pub mod dependencies;
#[cfg(feature = "storage-rocksdb")]
pub mod graph_cache;
mod health;
mod integrity;
mod jobs;
mod maintenance;
mod router;
mod types;

use std::sync::Arc;

/// Application state for management endpoints.
#[derive(Clone)]
pub struct ManagementState<S> {
    pub storage: Arc<S>,
}

pub use router::management_router;
