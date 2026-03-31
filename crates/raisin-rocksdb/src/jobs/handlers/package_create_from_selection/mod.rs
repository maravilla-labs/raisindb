//! Package create from selection job handler
//!
//! This module handles background creation of .rap packages from user-selected
//! content paths. It collects nodes from specified workspaces and paths,
//! generates a manifest, and creates a downloadable .rap file.

mod binary_helpers;
mod handler;
mod manifest_types;
mod node_loader;
mod package_builder;
mod types;

use super::package_install::{
    BinaryRetrievalCallback, BinaryStorageCallback, BinaryStorageFromPathCallback,
};
use crate::RocksDBStorage;
use raisin_storage::jobs::{JobId, JobRegistry};
use std::sync::Arc;

pub use types::{PackageCreateFromSelectionResult, SelectedPath};

/// Handler for package create from selection jobs
pub struct PackageCreateFromSelectionHandler {
    pub(super) storage: Arc<RocksDBStorage>,
    pub(super) job_registry: Arc<JobRegistry>,
    pub(super) binary_store_callback: Option<BinaryStorageCallback>,
    pub(super) binary_store_from_path_callback: Option<BinaryStorageFromPathCallback>,
    pub(super) binary_retrieval_callback: Option<BinaryRetrievalCallback>,
}

impl PackageCreateFromSelectionHandler {
    /// Create a new handler
    pub fn new(storage: Arc<RocksDBStorage>, job_registry: Arc<JobRegistry>) -> Self {
        Self {
            storage,
            job_registry,
            binary_store_callback: None,
            binary_store_from_path_callback: None,
            binary_retrieval_callback: None,
        }
    }

    /// Set the binary storage callback
    pub fn with_binary_store_callback(mut self, callback: BinaryStorageCallback) -> Self {
        self.binary_store_callback = Some(callback);
        self
    }

    /// Set the binary storage from path callback (for large files)
    pub fn with_binary_store_from_path_callback(
        mut self,
        callback: BinaryStorageFromPathCallback,
    ) -> Self {
        self.binary_store_from_path_callback = Some(callback);
        self
    }

    /// Set the binary retrieval callback (for downloading embedded files)
    pub fn with_binary_retrieval_callback(mut self, callback: BinaryRetrievalCallback) -> Self {
        self.binary_retrieval_callback = Some(callback);
        self
    }

    /// Report progress to job registry
    pub(super) async fn report_progress(&self, job_id: &JobId, progress: f32, message: &str) {
        tracing::debug!(job_id = %job_id, progress = %progress, message = %message, "Package creation progress");
        if let Err(e) = self.job_registry.update_progress(job_id, progress).await {
            tracing::warn!(job_id = %job_id, error = %e, "Failed to update job progress");
        }
    }
}
