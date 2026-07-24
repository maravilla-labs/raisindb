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
    definitions: &Arc<raisin_core::definitions::DefinitionResolver>,
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
            definitions.clone(),
        ),
    );

    if let Err(e) = builtin_handler.scan_existing_repositories().await {
        tracing::error!(error = %e, "Failed to scan existing repositories for builtin packages");
    }

    event_bus.subscribe(builtin_handler);
    tracing::info!("Builtin package init handler registered");
}

/// Re-sync built-in (`raisin:*`) NodeTypes and Workspaces into every EXISTING
/// repository so a schema change reaches repos created before the change.
///
/// The `RepositoryCreated` handler registers built-ins for NEW repos, but
/// nothing re-registered them for repos created earlier, so an updated
/// definition never reached an existing repository and the old, stricter schema
/// kept rejecting writes.
///
/// # Content hash, not `version:`
///
/// This used to call the version-gated `init_repository_nodetypes`, which only
/// wrote when the YAML's `version:` integer had been bumped. Editing a
/// definition and forgetting that bump was a silent no-op for every existing
/// tenant — the failure mode that cost a release cycle each time it was hit.
/// The resync now compares **content hashes** (the same ones the system-updates
/// view tracks), so any edit propagates. Safety comes from classification
/// instead: breaking changes are withheld under the default
/// `AutoApplyPolicy::NonBreaking` and surface as pending updates in the admin
/// console. Unchanged definitions are a pure no-op, as before.
///
/// `definitions` is the resolved definition stack, so an overlay or
/// registry-fetched definition rolls out through exactly this path too.
#[cfg(feature = "storage-rocksdb")]
pub async fn resync_system_definitions(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    definitions: &raisin_core::definitions::DefinitionResolver,
    policy: raisin_core::system_updates::AutoApplyPolicy,
) {
    use raisin_core::system_updates::{resync_repository_definitions, ResyncOutcome};

    if policy == raisin_core::system_updates::AutoApplyPolicy::Off {
        tracing::info!("System definition resync disabled (auto_apply = off)");
        return;
    }

    let repos = match storage.repository_management().list_repositories().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "System definition resync: failed to list repositories");
            return;
        }
    };

    let system_update_repo = raisin_rocksdb::SystemUpdateRepositoryImpl::new(storage.db().clone());
    let nodetypes = definitions.nodetypes();
    let workspaces = definitions.workspaces();
    let mut total = ResyncOutcome::default();

    for repo_info in repos {
        let branch = repo_info.config.default_branch.clone();
        match resync_repository_definitions(
            storage.clone(),
            &system_update_repo,
            &repo_info.tenant_id,
            &repo_info.repo_id,
            &branch,
            &nodetypes,
            &workspaces,
            policy,
        )
        .await
        {
            Ok(outcome) => {
                total.applied += outcome.applied;
                total.pending += outcome.pending;
                total.unchanged += outcome.unchanged;
            }
            Err(e) => tracing::warn!(
                tenant_id = %repo_info.tenant_id,
                repo_id = %repo_info.repo_id,
                branch = %branch,
                error = %e,
                "System definition resync failed for repository"
            ),
        }
    }

    tracing::info!(
        applied = total.applied,
        pending = total.pending,
        unchanged = total.unchanged,
        ?policy,
        "System definition resync complete"
    );
}
