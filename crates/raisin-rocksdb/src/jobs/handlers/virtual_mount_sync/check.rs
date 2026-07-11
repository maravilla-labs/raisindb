//! Periodic scan: find due mounts and enqueue per-mount sync jobs.
//!
//! Mirrors `ScheduledTriggerCheck`. Enumerates tenants → repos → the
//! `raisin:system` workspace → enabled `raisin:VirtualMount` nodes, and enqueues
//! a `VirtualMountSync { mount_id, mode: "delta" }` for every mount that is due
//! (`last_sync_at + effective_interval <= now`) and not webhook-driven. Uses
//! `register_job_with_id_idempotent` with dedup key `vmount-sync:{mount_id}` so a
//! mount never has two in-flight syncs.

use std::sync::Arc;

use chrono::Utc;
use raisin_core::services::node_service::NodeService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_storage::jobs::{JobContext, JobId, JobType};
use raisin_storage::{RepositoryManagementRepository, Storage};

use super::config::{MountConfig, SYSTEM_WORKSPACE};
use crate::RocksDBStorage;

/// Scan mounts and enqueue those that are due. Returns the number enqueued.
pub async fn run_check(
    storage: &Arc<RocksDBStorage>,
    tenant_filter: Option<String>,
    repo_filter: Option<String>,
) -> Result<usize> {
    let now = Utc::now().timestamp();
    let tenants = match &tenant_filter {
        Some(t) => vec![t.clone()],
        None => crate::management::list_tenants(storage).await?,
    };

    let mut enqueued = 0usize;
    for tenant in tenants {
        let repos = match &repo_filter {
            Some(r) => vec![r.clone()],
            None => match crate::management::list_repositories(storage, &tenant).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, error = %e, "vmount-check: list repos failed");
                    continue;
                }
            },
        };
        for repo in repos {
            match check_repo(storage, &tenant, &repo, now).await {
                Ok(n) => enqueued += n,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, repo = %repo, error = %e, "vmount-check: repo scan failed")
                }
            }
        }
    }
    Ok(enqueued)
}

/// Enqueue due mounts for one repo.
async fn check_repo(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    now: i64,
) -> Result<usize> {
    // Mount/Integration CONFIG always lives on the repo's config (default)
    // branch. We scan ONLY this branch — never any forked branch. This is what
    // makes forked branches inert: a fork that copies a mount config node (with
    // its sync cursor + fencing token) never triggers a second sync, because the
    // scan never looks at it. The mount's `target_branch` selects where its
    // virtual nodes are materialized; it is not scanned for mounts.
    let branch = storage
        .repository_management()
        .get_repository(tenant, repo)
        .await
        .ok()
        .flatten()
        .map(|r| r.config.default_branch)
        .unwrap_or_else(|| "main".to_string());

    let svc: NodeService<RocksDBStorage> = NodeService::new_with_context(
        storage.clone(),
        tenant.to_string(),
        repo.to_string(),
        branch.clone(),
        SYSTEM_WORKSPACE.to_string(),
    )
    .with_auth(AuthContext::system());

    let mounts = svc.list_by_type("raisin:VirtualMount").await?;
    let mut enqueued = 0usize;
    for node in mounts {
        let mount = match MountConfig::from_node(&node) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(mount_id = %node.id, error = %e, "vmount-check: invalid mount");
                continue;
            }
        };
        if !is_due(&mount, now) {
            continue;
        }
        if enqueue_sync(storage, tenant, repo, &branch, &mount.mount_id).await? {
            enqueued += 1;
        }
    }
    Ok(enqueued)
}

/// Whether a mount should be synced now.
pub fn is_due(mount: &MountConfig, now: i64) -> bool {
    if !mount.enabled {
        return false;
    }
    if mount.state.status.as_deref() == Some("auth_required") {
        return false;
    }
    if mount.sync_config.mode == "webhook" {
        // Webhook mounts are push-driven, never polled — with ONE exception:
        // bootstrapping. A webhook mount that has no live push subscription
        // needs a single sync run to register one (the sync run's `ensure` step
        // subscribes, and does an initial delta). Once the subscription is
        // active the mount falls silent again and provider pings (via the
        // notifications endpoint → an on-demand delta sync) drive it thereafter.
        // A mount whose adapter can't push is marked `unsupported` and is never
        // re-bootstrapped (it would neither poll nor push; the admin must fix).
        return !mount.state.has_active_push(now)
            && mount.state.push_status.as_deref() != Some("unsupported");
    }
    match mount.state.last_sync_at {
        None => true,
        Some(last) => last + mount.effective_interval_secs() as i64 <= now,
    }
}

/// Idempotently enqueue a delta sync for one mount. Returns whether a new job
/// was registered (`false` when a sync is already in flight).
async fn enqueue_sync(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    mount_id: &str,
) -> Result<bool> {
    let job_type = JobType::VirtualMountSync {
        mount_id: mount_id.to_string(),
        mode: "delta".to_string(),
    };
    let job_id = JobId::new();
    let context = JobContext {
        tenant_id: tenant.to_string(),
        repo_id: repo.to_string(),
        branch: branch.to_string(),
        workspace_id: SYSTEM_WORKSPACE.to_string(),
        revision: raisin_hlc::HLC::now(),
        metadata: std::collections::HashMap::new(),
    };
    // Store context before the job becomes visible to dispatch.
    storage.job_data_store().put(&job_id, &context)?;

    let registered = storage
        .job_registry()
        .register_job_with_id_idempotent(
            job_id,
            job_type,
            tenant.to_string(),
            format!("vmount-sync:{mount_id}"),
            None,
        )
        .await?;
    Ok(registered)
}
