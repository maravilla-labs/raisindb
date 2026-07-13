//! Binary storage initialization and builtin package setup.

use std::sync::Arc;

use raisin_binary::BinaryStorage;
#[cfg(not(feature = "s3"))]
use raisin_binary::FilesystemBinaryStorage;
#[cfg(feature = "s3")]
use raisin_binary::S3BinaryStorage;
#[cfg(feature = "storage-rocksdb")]
use raisin_storage::{RepositoryManagementRepository, Storage};

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

/// Re-sync embedded global (`raisin:*`) NodeTypes into every EXISTING
/// repository so a schema change shipped in a new server binary propagates on
/// upgrade — not only to repositories created after the change.
///
/// The `RepositoryCreated` handler registers global NodeTypes for NEW repos,
/// but nothing re-registered them for repos created earlier. That meant a
/// version-bumped global NodeType (e.g. `raisin:InboxTask` relaxing a required
/// field so standalone `raisin.tasks.create` tasks validate) never reached an
/// existing repository, and the old, stricter schema kept rejecting writes.
///
/// `init_repository_nodetypes` is version-gated: it only writes when the
/// embedded NodeType version is newer than the one registered in the repo. So
/// this is a cheap, idempotent no-op on every startup unless a global NodeType
/// version actually increased.
#[cfg(feature = "storage-rocksdb")]
pub async fn resync_global_nodetypes(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    let repos = match storage.repository_management().list_repositories().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "Global NodeType resync: failed to list repositories");
            return;
        }
    };

    for repo_info in repos {
        let branch = repo_info.config.default_branch.clone();
        if let Err(e) = raisin_core::nodetype_init::init_repository_nodetypes(
            storage.clone(),
            &repo_info.tenant_id,
            &repo_info.repo_id,
            &branch,
        )
        .await
        {
            tracing::warn!(
                tenant_id = %repo_info.tenant_id,
                repo_id = %repo_info.repo_id,
                branch = %branch,
                error = %e,
                "Global NodeType resync failed for repository"
            );
        }
    }
    tracing::info!("Global NodeType resync complete");
}
