//! Package-related job handler construction
//!
//! Creates handlers for package install, processing, export,
//! and create-from-selection operations.

use std::sync::Arc;

use crate::jobs::handlers::package_install::AppliedHashRecorder;
use crate::jobs::{BinaryRetrievalCallback, BinaryStorageCallback, PackageInstallHandler};
use crate::repositories::SystemUpdateRepositoryImpl;
use crate::storage::RocksDBStorage;
use raisin_storage::jobs::JobRegistry;
use raisin_storage::{AppliedDefinition, ResourceType, SystemUpdateRepository};

/// Create the package install handler
pub fn create_package_install_handler(
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    binary_retrieval: Option<&BinaryRetrievalCallback>,
    binary_storage: Option<&BinaryStorageCallback>,
) -> Arc<PackageInstallHandler<RocksDBStorage>> {
    // Built here because this is where storage is CONCRETE: the applied-hash
    // record lives in a RocksDB repository while the handler is generic, which
    // is why it travels as a callback rather than a repository handle.
    //
    // What it buys: an on-demand install (the Integrations gallery) becomes
    // visible to the boot-time staleness check, so the next release updates the
    // package. Without it "no record" reads as "assume current" and a
    // gallery-installed connector never updates again — the binary carries the
    // fix and the content has never heard of it.
    let system_updates = SystemUpdateRepositoryImpl::new(storage.db().clone());
    let recorder: AppliedHashRecorder = Arc::new(move |tenant_id, repo_id, name, content_hash| {
        let repo = system_updates.clone();
        Box::pin(async move {
            repo.set_applied(
                &tenant_id,
                &repo_id,
                ResourceType::Package,
                &name,
                AppliedDefinition {
                    content_hash,
                    // Package versions are strings; this column is an i32 and
                    // is not what staleness compares.
                    applied_version: None,
                    applied_at: chrono::Utc::now(),
                    applied_by: "package-installer".to_string(),
                },
            )
            .await
        })
    });

    let mut builder =
        PackageInstallHandler::new(storage, job_registry).with_applied_recorder(recorder);
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
