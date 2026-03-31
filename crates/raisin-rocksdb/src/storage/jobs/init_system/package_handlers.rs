//! Package-related job handler construction
//!
//! Creates handlers for package install, processing, export,
//! and create-from-selection operations.

use std::sync::Arc;

use crate::jobs::{BinaryRetrievalCallback, BinaryStorageCallback, PackageInstallHandler};
use crate::storage::RocksDBStorage;
use raisin_storage::jobs::JobRegistry;

/// Create the package install handler
pub fn create_package_install_handler(
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    binary_retrieval: Option<&BinaryRetrievalCallback>,
    binary_storage: Option<&BinaryStorageCallback>,
) -> Arc<PackageInstallHandler<RocksDBStorage>> {
    let mut builder = PackageInstallHandler::new(storage, job_registry);
    if let Some(callback) = binary_retrieval {
        builder = builder.with_binary_callback(callback.clone());
    }
    if let Some(callback) = binary_storage {
        builder = builder.with_binary_store_callback(callback.clone());
    }
    Arc::new(builder)
}

/// Create the package process handler
pub fn create_package_process_handler(
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    binary_retrieval: Option<&BinaryRetrievalCallback>,
    binary_storage: Option<&BinaryStorageCallback>,
) -> Arc<crate::jobs::PackageProcessHandler<RocksDBStorage>> {
    let mut builder = crate::jobs::PackageProcessHandler::new(storage, job_registry);
    if let Some(callback) = binary_retrieval {
        builder = builder.with_binary_callback(callback.clone());
    }
    if let Some(callback) = binary_storage {
        builder = builder.with_binary_store_callback(callback.clone());
    }
    Arc::new(builder)
}

/// Create the package export handler
pub fn create_package_export_handler(
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    binary_storage: Option<&BinaryStorageCallback>,
) -> Arc<crate::jobs::PackageExportHandler<RocksDBStorage>> {
    let mut builder = crate::jobs::PackageExportHandler::new(storage, job_registry);
    if let Some(callback) = binary_storage {
        builder = builder.with_binary_store_callback(callback.clone());
    }
    Arc::new(builder)
}

/// Create the package create-from-selection handler
pub fn create_package_create_from_selection_handler(
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    binary_retrieval: Option<&BinaryRetrievalCallback>,
    binary_storage: Option<&BinaryStorageCallback>,
) -> Arc<crate::jobs::PackageCreateFromSelectionHandler> {
    let mut builder = crate::jobs::PackageCreateFromSelectionHandler::new(storage, job_registry);
    if let Some(callback) = binary_storage {
        builder = builder.with_binary_store_callback(callback.clone());
    }
    if let Some(callback) = binary_retrieval {
        builder = builder.with_binary_retrieval_callback(callback.clone());
    }
    Arc::new(builder)
}
