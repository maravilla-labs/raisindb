//! Branch repository implementation
//!
//! This module provides branch management functionality including:
//! - Basic CRUD operations (create, read, update, delete)
//! - HEAD pointer management
//! - Branch divergence calculation (ahead/behind commits)
//! - Merge conflict detection
//! - Merge operations (fast-forward and three-way)
//! - Branch index copying for efficient branch creation

mod cf_registry;
mod conflict;
mod copy;
mod crud;
mod diff;
mod divergence;
mod head;
mod merge;

pub(crate) use head::lock_branch_record;

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
    /// Publishes `RepositoryEventKind::BranchCreated`, which is what enqueues
    /// the fulltext and embedding branch-copy jobs. Without it a new branch
    /// has its records but no derived indexes until a rebuild.
    pub(crate) event_bus: Option<Arc<dyn raisin_events::EventBus>>,
}

impl BranchRepositoryImpl {
    /// Create a new branch repository instance
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            operation_capture: None,
            job_registry: None,
            job_data_store: None,
            event_bus: None,
        }
    }

    /// Create a new branch repository with operation capture for replication
    pub fn new_with_capture(db: Arc<DB>, operation_capture: Arc<crate::OperationCapture>) -> Self {
        Self {
            db,
            operation_capture: Some(operation_capture),
            job_registry: None,
            job_data_store: None,
            event_bus: None,
        }
    }

    /// Set the event bus on which branch lifecycle events are published.
    pub fn with_event_bus(mut self, event_bus: Arc<dyn raisin_events::EventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
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
