//! Package create from selection: a streaming build of a `.rap` archive.
//!
//! The selection is walked page by page and written straight into the blob
//! store, with the archive's sha256 and size, a sorted inventory and the
//! manifest produced in the same pass, and progress mirrored onto a
//! caller-named build record. See `handler.rs` for the phases, `mirror.rs`
//! for the record contract and `submit.rs` for the entry points the HTTP
//! layer and the function binding share.

mod archive;
mod estimate;
mod handler;
mod inventory;
pub mod ledger;
mod manifest_types;
mod mirror;
mod progress;
mod registration;
mod request;
pub mod scope;
pub(crate) mod submit;
#[cfg(test)]
mod tests;
mod types;

use super::package_install::{
    BinaryRetrievalCallback, BinaryStorageCallback, BinaryStorageFromPathCallback,
};
use crate::RocksDBStorage;
use raisin_storage::jobs::{JobId, JobRegistry};
use std::sync::Arc;

pub use mirror::RecordRef;
pub use submit::{
    cancel_package_build, name_is_reserved, name_is_valid, package_build_status,
    submit_package_build, BuildStatus, PackageBuildRequest, RecordRefWire, Submitted, CANCEL_GRACE,
};
pub use types::{PackageCreateFromSelectionResult, SelectedPath};

/// Handler for package create from selection jobs
pub struct PackageCreateFromSelectionHandler {
    pub(super) storage: Arc<RocksDBStorage>,
    pub(super) job_registry: Arc<JobRegistry>,
    pub(super) binary_store_callback: Option<BinaryStorageCallback>,
    pub(super) binary_store_from_path_callback: Option<BinaryStorageFromPathCallback>,
    pub(super) binary_retrieval_callback: Option<BinaryRetrievalCallback>,
    /// Replaces the free-space floor (see `estimate::floor_override`).
    pub(super) space_floor: Option<u64>,
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
            space_floor: None,
        }
    }

    /// Replace the free-space safety floor with a fixed number of bytes.
    pub fn with_space_floor(mut self, floor: Option<u64>) -> Self {
        self.space_floor = floor;
        self
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
