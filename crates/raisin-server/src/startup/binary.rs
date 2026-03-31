//! Binary storage initialization and builtin package setup.

use std::sync::Arc;

use raisin_binary::BinaryStorage;
#[cfg(not(feature = "s3"))]
use raisin_binary::FilesystemBinaryStorage;
#[cfg(feature = "s3")]
use raisin_binary::S3BinaryStorage;
#[cfg(feature = "storage-rocksdb")]
use raisin_storage::Storage;

/// Initialize the binary storage backend based on feature flags.
#[cfg(feature = "s3")]
pub async fn init_binary_storage() -> Arc<dyn BinaryStorage> {
    let bin = Arc::new(
        S3BinaryStorage::from_env()
            .await
            .expect("S3BinaryStorage config"),
    );
    tracing::info!("Binary storage initialized (S3)");
    bin
}

/// Initialize the binary storage backend based on feature flags.
#[cfg(not(feature = "s3"))]
pub fn init_binary_storage(data_dir: &str) -> Arc<FilesystemBinaryStorage> {
    let upload_path = std::path::Path::new(data_dir).join("uploads");
    let bin = Arc::new(FilesystemBinaryStorage::new(
        &upload_path,
        Some("/files".into()),
    ));
    tracing::info!(path = %upload_path.display(), "Binary storage initialized (filesystem)");
    bin
}

/// Register the builtin package init handler and scan existing repositories.
#[cfg(feature = "storage-rocksdb")]
pub async fn register_builtin_package_handler<B: BinaryStorage + 'static>(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    bin: &Arc<B>,
) {
    use crate::builtin_package_init_handler;

    let event_bus = storage.event_bus();
    let system_update_repo = raisin_rocksdb::SystemUpdateRepositoryImpl::new(storage.db().clone());
    let builtin_handler = Arc::new(
        builtin_package_init_handler::BuiltinPackageInitHandler::new(
            storage.clone(),
            bin.clone(),
            storage.job_registry().clone(),
            storage.job_data_store().clone(),
            system_update_repo,
        ),
    );

    if let Err(e) = builtin_handler.scan_existing_repositories().await {
        tracing::error!(error = %e, "Failed to scan existing repositories for builtin packages");
    }

    event_bus.subscribe(builtin_handler);
    tracing::info!("Builtin package init handler registered");
}
