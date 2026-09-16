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

/// Queue a build for every declared compound index with no usable build
/// state, across every tenant/repo/branch/workspace.
///
/// The only other producers of this queue are a NodeType create/update event
/// and a workspace create event (`jobs/event_handler/delete_and_schema_handlers.rs`)
/// — so an index whose declaring event fired before the binary that first
/// tracked build state, or whose state was lost (`cf::INDEX_STATUS` is
/// excluded from branch copy), has no event left to ever re-fire it, and the
/// fail-closed planner gate answers `NotBuilt` forever with nothing to notice.
/// Spawned rather than awaited: enumeration is cheap, but a real backlog of
/// first-time builds must not hold up the rest of startup.
#[cfg(feature = "storage-rocksdb")]
pub fn sweep_compound_indexes_at_boot(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    let storage = storage.clone();
    tokio::spawn(async move {
        match raisin_rocksdb::management::sweep_compound_index_builds_at_boot(&storage).await {
            Ok(0) => {
                tracing::debug!("Boot compound-index sweep: every declared index already usable")
            }
            Ok(queued) => tracing::info!(queued, "Boot compound-index sweep: queued builds"),
            Err(e) => tracing::warn!(error = %e, "Boot compound-index sweep failed"),
        }
    });
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
    // `list_branches` lives on this trait; without it in scope the resync can
    // only ever see the default branch.
    use raisin_storage::BranchRepository;

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
        // EVERY branch, not only the default one.
        //
        // The type registry is per branch while builtin types are global, so a
        // resync that reached `main` alone left every other branch frozen at the
        // definitions it was forked with. `publish` is the branch that matters:
        // each release installs a content-less package onto it to register
        // types, and that install needs `raisin:Package` to exist there. It did
        // not — so the step failed with `NodeType 'raisin:Package' not found` on
        // every project on every run, swallowed by the pipelines' `|| echo`.
        // Confirmed on two unrelated repositories on 2026-09-16.
        //
        // If branches cannot be listed the default one is still tried, so a
        // storage backend without branch listing behaves exactly as before.
        let branches = match storage
            .branches()
            .list_branches(&repo_info.tenant_id, &repo_info.repo_id)
            .await
        {
            Ok(list) if !list.is_empty() => {
                list.into_iter().map(|b| b.name).collect::<Vec<String>>()
            }
            Ok(_) => vec![repo_info.config.default_branch.clone()],
            Err(e) => {
                tracing::warn!(
                    tenant_id = %repo_info.tenant_id,
                    repo_id = %repo_info.repo_id,
                    error = %e,
                    "System definition resync: could not list branches, using the default branch"
                );
                vec![repo_info.config.default_branch.clone()]
            }
        };

        for branch in branches {
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
    }

    tracing::info!(
        applied = total.applied,
        pending = total.pending,
        unchanged = total.unchanged,
        ?policy,
        "System definition resync complete"
    );
}
