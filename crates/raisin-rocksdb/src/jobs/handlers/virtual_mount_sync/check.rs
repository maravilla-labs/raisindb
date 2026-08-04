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
    // An unfinished backfill is due as soon as the previous chunk finished,
    // rather than after the poll interval: a mailbox imported 500 items per run
    // at one run every 5 minutes would take days. Chunks therefore run
    // back-to-back — and because each run does its delta pass FIRST (see
    // `run_sync`), new items keep arriving throughout. Gated on
    // consecutive_failures so a broken backfill cannot spin.
    //
    // THIS MUST STAY ABOVE THE WEBHOOK BRANCH. It used to sit below it, which
    // made it unreachable for `mode: "webhook"`: a webhook mount with a live
    // subscription is never due, so its half-finished import advanced only when
    // the provider happened to send a ping. A quiet mailbox meant an import that
    // stopped at the first chunk and stayed there — a production mount sat at
    // 500 of ~8,000 items for hours reporting `status: "ok"`. Draining a
    // backfill is our own unfinished work; whether the provider can push has
    // nothing to do with it.
    let backfill_pending =
        !mount.state.backfill_stack.is_empty() || mount.state.backfill_cursor.is_some();
    if backfill_pending && mount.state.consecutive_failures == 0 {
        return true;
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

    // Measure from the last ATTEMPT, not the last success. `last_sync_at` only
    // advances when a sync succeeds, so scheduling on it alone left a failing
    // mount permanently "due": the backoff in `effective_interval_secs` was
    // computed against a timestamp frozen at the last good run and could never
    // catch up to `now`. A mount with a bad remote root therefore retried on
    // every tick, forever, against the provider's API.
    let last_activity = match (mount.state.last_sync_at, mount.state.last_attempt_at) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    match last_activity {
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
        trigger: "schedule".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::handlers::virtual_mount_sync::config::{MountState, SyncConfig, WriteConfig};

    const INTERVAL: u64 = 300;

    fn mount() -> MountConfig {
        MountConfig {
            mount_id: "m1".into(),
            integration_ref: "/integrations/ms-graph".into(),
            account_ref: None,
            target_workspace: "default".into(),
            target_branch: "main".into(),
            mount_path: "/mail".into(),
            remote_root: None,
            adapter_function: None,
            mapping_function: None,
            enabled: true,
            sync_config: SyncConfig {
                interval_seconds: INTERVAL,
                ..Default::default()
            },
            sync_config_raw: serde_json::json!({}),
            write_config: WriteConfig::default(),
            state: MountState::default(),
        }
    }

    #[test]
    fn a_never_synced_mount_is_due() {
        assert!(is_due(&mount(), 1_000_000));
    }

    #[test]
    fn a_recent_success_is_not_due_until_the_interval_elapses() {
        let mut m = mount();
        m.state.last_sync_at = Some(1_000_000);
        assert!(!is_due(&m, 1_000_000 + INTERVAL as i64 - 1));
        assert!(is_due(&m, 1_000_000 + INTERVAL as i64));
    }

    /// The regression this whole change exists for. A mount that succeeded once
    /// and then began failing kept a frozen `last_sync_at`, so the backed-off
    /// interval was always already in the past and the scheduler re-enqueued it
    /// on EVERY tick — observed in production as a broken mount retrying against
    /// Microsoft Graph every couple of minutes, indefinitely.
    #[test]
    fn a_failing_mount_backs_off_from_its_last_attempt() {
        let mut m = mount();
        m.state.last_sync_at = Some(1_000_000); // last success: long ago
        m.state.consecutive_failures = 5; // interval * 2^5
        let now = 2_000_000; // far past last_sync_at + any backoff

        // Attempted just now: must NOT be due again yet.
        m.state.last_attempt_at = Some(now);
        assert!(
            !is_due(&m, now + 10),
            "a mount that just failed must wait out its backoff"
        );

        // Once the backed-off interval has elapsed since the attempt, it is due.
        let backoff = INTERVAL as i64 * 32;
        assert!(is_due(&m, now + backoff));
    }

    /// Belt and braces: whichever timestamp is later wins, so an out-of-order
    /// write cannot make a mount look permanently due.
    #[test]
    fn the_later_timestamp_wins() {
        let mut m = mount();
        m.state.last_sync_at = Some(5_000);
        m.state.last_attempt_at = Some(1_000);
        assert!(!is_due(&m, 5_000 + INTERVAL as i64 - 1));

        m.state.last_sync_at = Some(1_000);
        m.state.last_attempt_at = Some(5_000);
        assert!(!is_due(&m, 5_000 + INTERVAL as i64 - 1));
    }

    #[test]
    fn disabled_and_auth_required_mounts_are_never_due() {
        let mut m = mount();
        m.enabled = false;
        assert!(!is_due(&m, 9_999_999));

        let mut m = mount();
        m.state.status = Some("auth_required".into());
        assert!(!is_due(&m, 9_999_999));
    }
}
