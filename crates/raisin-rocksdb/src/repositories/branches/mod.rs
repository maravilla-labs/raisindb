//! Branch repository implementation
//!
//! This module provides branch management functionality including:
//! - Basic CRUD operations (create, read, update, delete)
//! - HEAD pointer management
//! - Branch divergence calculation (ahead/behind commits)
//! - Merge conflict detection
//! - Merge operations (fast-forward and three-way)
//! - Branch index copying for efficient branch creation

mod conflict;
mod copy;
mod crud;
mod divergence;
mod head;
mod merge;

use crate::jobs::JobDataStore;
use raisin_storage::jobs::JobRegistry;
use rocksdb::DB;
use std::sync::Arc;

/// Branch repository implementation using RocksDB
///
/// Provides all branch management operations including versioning,
/// merging, and conflict resolution.
#[derive(Clone)]
pub struct BranchRepositoryImpl {
    pub(crate) db: Arc<DB>,
    pub(crate) operation_capture: Option<Arc<crate::OperationCapture>>,
    pub(crate) job_registry: Option<Arc<JobRegistry>>,
    pub(crate) job_data_store: Option<Arc<JobDataStore>>,
}

impl BranchRepositoryImpl {
    /// Create a new branch repository instance
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            operation_capture: None,
            job_registry: None,
            job_data_store: None,
        }
    }

    /// Create a new branch repository with operation capture for replication
    pub fn new_with_capture(db: Arc<DB>, operation_capture: Arc<crate::OperationCapture>) -> Self {
        Self {
            db,
            operation_capture: Some(operation_capture),
            job_registry: None,
            job_data_store: None,
        }
    }

    /// Set the job registry and data store for background job enqueueing
    pub fn with_job_system(
        mut self,
        job_registry: Arc<JobRegistry>,
        job_data_store: Arc<JobDataStore>,
    ) -> Self {
        self.job_registry = Some(job_registry);
        self.job_data_store = Some(job_data_store);
        self
    }
}
