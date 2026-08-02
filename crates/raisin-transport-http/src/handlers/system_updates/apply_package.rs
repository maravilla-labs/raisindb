//! Package update application logic.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::Utc;
use raisin_core::definitions::PackageDefinition;
use raisin_core::package_registration::{
    self, PackageNodeStore, PackageOrigin, PackageRegistration, PackageResource, PACKAGES_WORKSPACE,
};
use raisin_models::nodes::Node;
use raisin_packages::Manifest;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::system_updates::{AppliedDefinition, PendingUpdate, ResourceType};
use raisin_storage::{NodeRepository, Storage, StorageScope, SystemUpdateRepository};

use crate::error::ApiError;
use crate::state::AppState;

use super::map_storage_err;

/// [`PackageNodeStore`] over the plain node repository.
///
/// The system-update apply endpoint has no transaction of its own; it writes
/// through `storage().nodes()`. Only the write mechanism lives here — the node
/// id, property shape and install mode are
/// [`raisin_core::package_registration`]'s.
struct RepoPackageStore<'a> {
    state: &'a AppState,
    tenant_id: &'a str,
    repo_id: &'a str,
    branch: &'a str,
}

impl<'a> RepoPackageStore<'a> {
    fn scope(&self) -> StorageScope<'a> {
        StorageScope::new(
            self.tenant_id,
            self.repo_id,
            self.branch,
            PACKAGES_WORKSPACE,
        )
    }
}

#[async_trait]
impl PackageNodeStore for RepoPackageStore<'_> {
    async fn get_package_node_by_path(&self, path: &str) -> anyhow::Result<Option<Node>> {
        self.state
            .storage()
            .nodes()
            .get_by_path(self.scope(), path, None)
            .await
            .map_err(anyhow::Error::new)
    }

    async fn write_package_node(&self, node: Node, existing: Option<&Node>) -> anyhow::Result<()> {
        let nodes = self.state.storage().nodes();
        if existing.is_some() {
            nodes
                .update(
                    self.scope(),
                    node,
                    raisin_storage::node_operations::UpdateNodeOptions::default(),
                )
                .await
                .map_err(anyhow::Error::new)?;
        } else {
            nodes
                .create(
                    self.scope(),
                    node,
                    raisin_storage::node_operations::CreateNodeOptions::default(),
                )
                .await
                .map_err(anyhow::Error::new)?;
        }
        Ok(())
    }
}

/// Apply a single Package system update.
///
/// Creates a ZIP from the package's files (embedded or overlay), stores the
/// binary, registers the package node and queues an installation job.
pub(super) async fn apply_package_update(
    state: &AppState,
    system_update_repo: &impl SystemUpdateRepository,
    rocksdb: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    update: &PendingUpdate,
    packages: &[PackageDefinition],
) -> Result<bool, ApiError> {
    let Some(package) = packages
        .iter()
        .find(|p| p.info.manifest.name == update.name)
    else {
        return Ok(false);
    };
    let package_info = &package.info;
    let manifest: &Manifest = &package_info.manifest;

    let store = RepoPackageStore {
        state,
        tenant_id,
        repo_id,
        branch,
    };

    // No already-installed short-circuit here, deliberately. Unlike the boot
    // scan — which runs unprompted for every package in every repo and must not
    // re-zip the world — this path only runs for updates
    // `check_pending_updates` has already flagged as pending, and only when an
    // operator explicitly asked for them to be applied. Re-applying is the
    // requested action.
    let zip_data = package
        .source
        .to_zip()
        .map_err(|e| ApiError::internal(format!("Failed to create package ZIP: {}", e)))?;

    // Store the `.rap`, always inside the tenant's blob namespace. This used to
    // pass `None`, which put the blob outside the tenant prefix where it
    // escapes per-tenant accounting and cleanup.
    let stored = package_registration::store_package_blob(
        state.bin.as_ref(),
        tenant_id,
        manifest,
        &zip_data,
    )
    .await
    .map_err(|e| ApiError::internal(format!("Failed to store package file: {}", e)))?;

    let registration = PackageRegistration {
        manifest,
        resource: PackageResource::from_stored(&stored, zip_data.len()),
        origin: PackageOrigin::Builtin {
            content_hash: package_info.content_hash.clone(),
        },
    };

    let registered = package_registration::register_package_node(&store, &registration)
        .await
        .map_err(|e| map_storage_err("Failed to register package node", storage_err(e)))?;

    // Queue installation job. `sync` for a package already registered here,
    // `skip` for a first install — deliberately never `overwrite`, which
    // bypasses the package's own `manifest.yaml` `sync:` policy and
    // deletes/replaces content the package author marked as user-owned. Forcing
    // an overwrite stays an explicit operator action via
    // `POST /packages/{name}/install?mode=overwrite`.
    let job_id = queue_install_job(
        rocksdb,
        manifest,
        &registered.node_id,
        &stored.key,
        registered.install_mode(),
        tenant_id,
        repo_id,
        branch,
    )
    .await?;

    // Record the applied hash
    system_update_repo
        .set_applied(
            tenant_id,
            repo_id,
            ResourceType::Package,
            &manifest.name,
            AppliedDefinition {
                content_hash: package_info.content_hash.clone(),
                applied_version: None, // Package version is a string, not i32
                applied_at: Utc::now(),
                applied_by: "admin".to_string(),
            },
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to record applied hash: {}", e)))?;

    tracing::info!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        package = %update.name,
        node_id = %registered.node_id,
        install_mode = %registered.install_mode(),
        job_id = %job_id,
        "Applied Package system update and queued installation job"
    );

    Ok(true)
}

/// The store adapter surfaces `anyhow` errors; recover the original
/// `raisin_error::Error` so `map_storage_err` can still turn a `NotFound` into
/// a 404 rather than flattening it to an opaque 500.
fn storage_err(e: anyhow::Error) -> raisin_error::Error {
    match e.downcast::<raisin_error::Error>() {
        Ok(inner) => inner,
        Err(other) => raisin_error::Error::storage(other.to_string()),
    }
}

/// Queue a package installation job via the unified job queue.
#[allow(clippy::too_many_arguments)]
async fn queue_install_job(
    rocksdb: &RocksDBStorage,
    manifest: &Manifest,
    node_id: &str,
    resource_key: &str,
    install_mode: &str,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<String, ApiError> {
    let job_type = raisin_storage::JobType::PackageInstall {
        package_name: manifest.name.clone(),
        package_version: manifest.version.clone(),
        package_node_id: node_id.to_string(),
    };

    let mut metadata = HashMap::new();
    metadata.insert(
        "resource_key".to_string(),
        serde_json::Value::String(resource_key.to_string()),
    );
    metadata.insert(
        "install_mode".to_string(),
        serde_json::Value::String(install_mode.to_string()),
    );

    let job_context = raisin_storage::jobs::JobContext {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch.to_string(),
        workspace_id: PACKAGES_WORKSPACE.to_string(),
        revision: raisin_hlc::HLC::now(),
        metadata,
    };

    let job_registry = rocksdb.job_registry();
    let job_data_store = rocksdb.job_data_store();

    // Store job context BEFORE registering so dispatch can never observe
    // the job without its context.
    let job_id = raisin_storage::jobs::JobId::new();
    job_data_store
        .put(&job_id, &job_context)
        .map_err(|e| ApiError::internal(format!("Failed to store job context: {}", e)))?;

    job_registry
        .register_job_with_id(
            job_id.clone(),
            job_type,
            tenant_id.to_string(),
            None,
            None,
            None,
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to register job: {}", e)))?;

    Ok(job_id.to_string())
}
