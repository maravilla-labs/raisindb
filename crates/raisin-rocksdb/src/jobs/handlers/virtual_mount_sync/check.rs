//! Periodic scan: find due mounts and enqueue per-mount sync jobs.
//!
//! Mirrors `ScheduledTriggerCheck`. Enumerates tenants → repos → the
//! `raisin:system` workspace → enabled `raisin:VirtualMount` nodes, and enqueues
//! a `VirtualMountSync { mount_id, mode: "delta" }` for every mount that is due
//! (`last_sync_at + effective_interval <= now`) and not webhook-driven. Uses
//! `register_job_with_id_idempotent` with dedup key `vmount-sync:{mount_id}` so a
//! mount never has two in-flight syncs.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use chrono::Utc;
use raisin_core::services::node_service::NodeService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_storage::jobs::{JobContext, JobId, JobType};
use raisin_storage::{RepositoryManagementRepository, Storage};

use super::config::{MountConfig, SYSTEM_WORKSPACE};
use crate::RocksDBStorage;

/// Every `(tenant, repo)` a periodic mount scan should visit.
///
/// Shared by the sync check and the write-reconcile walk so the two cannot
/// disagree about which repositories exist — a scan that enumerates differently
/// from its sibling is a mount that is synced but never reconciled, or the
/// reverse, with nothing anywhere saying so.
///
/// # This is O(repos WITH mounts), not O(all repos)
///
/// It used to be the latter: `list_tenants` × `list_repositories`, then a
/// `list_by_type("raisin:VirtualMount")` per repo in [`check_repo`]. Both of
/// those are index-backed and individually cheap; the problem was volume. Every
/// 60 seconds the tick fanned a type scan out to every repository in the
/// deployment, virtually none of which had ever had a mount, and
/// `VirtualMountSyncHandler::handle` measured **51% of total server CPU** in
/// production. The shape dates to the feature's first commit (`e201f92`), so
/// there was no regression to find — the enumeration itself was the cost.
///
/// [`crate::vmount_registry`] now records which repos hold a mount, written in
/// the same batch as the mount node, and this reads that. An EXPLICIT
/// `repo_filter` still bypasses the registry: an operator naming a repository
/// means "look here", and answering "that repo isn't in the index" would make a
/// targeted invocation the one call that cannot repair anything.
///
/// The registry being complete is load-bearing, so it is audited — see
/// [`super::registry_backfill`], driven from [`run_check`].
pub(super) async fn scan_scope(
    storage: &Arc<RocksDBStorage>,
    tenant_filter: Option<String>,
    repo_filter: Option<String>,
) -> Result<Vec<(String, String)>> {
    let tenants = match &tenant_filter {
        Some(t) => vec![t.clone()],
        None => crate::management::list_tenants(storage).await?,
    };
    let mut out = Vec::new();
    for tenant in tenants {
        let repos = match &repo_filter {
            Some(r) => vec![r.clone()],
            None => match crate::management::list_repos_with_virtual_mounts(storage, &tenant).await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, error = %e, "vmount scan: list repos failed");
                    continue;
                }
            },
        };
        out.extend(repos.into_iter().map(|repo| (tenant.clone(), repo)));
    }
    Ok(out)
}

/// How often the full-enumeration audit of the registry runs.
const REGISTRY_RECONCILE_INTERVAL_SECS: i64 = 3600;

/// When this process last ran the audit. `0` means never, which is what makes
/// the first tick after startup the BACKFILL pass for mounts predating the
/// registry.
///
/// Process-local on purpose. The registry is derived local state, like the
/// spatial and fulltext indexes: each node writes its own from its own node
/// writes, so each node must audit its own. A cluster-wide lease would leave
/// every other node's registry unaudited.
static LAST_REGISTRY_RECONCILE: AtomicI64 = AtomicI64::new(0);

/// Run the registry audit if it is due, swallowing failures.
///
/// An audit that fails must not stop the tick it rides on: the fast path is
/// still correct for every mount that IS registered, and refusing to sync those
/// because the drift check errored would turn a diagnostic into an outage.
async fn maybe_reconcile_registry(
    storage: &Arc<RocksDBStorage>,
    tenant_filter: &Option<String>,
    repo_filter: &Option<String>,
    now: i64,
) {
    let last = LAST_REGISTRY_RECONCILE.load(Ordering::Relaxed);
    if last != 0 && now - last < REGISTRY_RECONCILE_INTERVAL_SECS {
        return;
    }
    // Claim the slot before doing the work, so two overlapping ticks in this
    // process don't both walk every repository.
    if LAST_REGISTRY_RECONCILE
        .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let first_pass = last == 0;

    match super::registry_backfill::reconcile_registry(
        storage,
        tenant_filter.clone(),
        repo_filter.clone(),
    )
    .await
    {
        Ok(report) => {
            if first_pass {
                tracing::info!(
                    repos = report.repos_scanned,
                    mounts = report.mounts_found,
                    backfilled = report.repaired,
                    pruned = report.pruned,
                    "vmount-registry: backfill pass complete"
                );
            } else if report.repaired > 0 {
                // Loud on purpose: after startup, a repair means a node write
                // path staged a mount without staging its registry entry, and
                // that mount was not syncing until this pass found it.
                tracing::warn!(
                    repos = report.repos_scanned,
                    mounts = report.mounts_found,
                    repaired = report.repaired,
                    "vmount-registry: DRIFT — the fast sync scan was missing mounts"
                );
            } else {
                tracing::debug!(
                    repos = report.repos_scanned,
                    mounts = report.mounts_found,
                    pruned = report.pruned,
                    "vmount-registry: reconcile clean"
                );
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "vmount-registry: reconcile failed");
        }
    }
}

/// The branch a repo's mount/integration CONFIG lives on.
///
/// Always the repo's default branch, never a fork. This is what makes forked
/// branches inert: a fork that copies a mount config node (with its sync cursor
/// and fencing token) never triggers a second sync, because no scan looks at it.
/// The mount's `target_branch` selects where its virtual nodes materialize; it
/// is not scanned for mounts.
///
/// `pub(crate)` rather than `pub(super)` because the event-bus capture hook
/// resolves the same branch to address a mount, and a second implementation of
/// "which branch is a mount's config on" is exactly the mirrored-path drift that
/// would have it enqueue jobs against a branch no scan ever reads.
pub(crate) async fn config_branch(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
) -> String {
    storage
        .repository_management()
        .get_repository(tenant, repo)
        .await
        .ok()
        .flatten()
        .map(|r| r.config.default_branch)
        .unwrap_or_else(|| "main".to_string())
}

/// Scan mounts and enqueue those that are due. Returns the number enqueued.
pub async fn run_check(
    storage: &Arc<RocksDBStorage>,
    tenant_filter: Option<String>,
    repo_filter: Option<String>,
) -> Result<usize> {
    let now = Utc::now().timestamp();

    // Before the fast scan, not after: a repair found here must take effect on
    // THIS tick, or a stranded mount waits another minute for no reason.
    maybe_reconcile_registry(storage, &tenant_filter, &repo_filter, now).await;

    let mut enqueued = 0usize;
    for (tenant, repo) in scan_scope(storage, tenant_filter, repo_filter).await? {
        match check_repo(storage, &tenant, &repo, now).await {
            Ok(n) => enqueued += n,
            Err(e) => {
                tracing::warn!(tenant = %tenant, repo = %repo, error = %e, "vmount-check: repo scan failed")
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
    let branch = config_branch(storage, tenant, repo).await;

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
                // A `warn` and `continue` was the whole response here, and it
                // is not one: the mount is now skipped on every tick, in BOTH
                // directions (a corrupt `write_config` stops reads too), while
                // `state.status` keeps whatever the last good run wrote and
                // `last_attempt_at` freezes. Nothing in the console changes.
                // Record the verdict where an operator looks — the same
                // channel a bad `target_branch` uses.
                tracing::warn!(mount_id = %node.id, error = %e, "vmount-check: invalid mount");
                if let Err(werr) = super::misconfig::mark_unparseable_mount(
                    storage, tenant, repo, &branch, &node.id, &e,
                )
                .await
                {
                    tracing::warn!(
                        mount_id = %node.id,
                        error = %werr,
                        "vmount-check: could not record the misconfigured verdict"
                    );
                }
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
    // Paused by an operator. Distinct from `!enabled`, which also tears the push
    // subscription down — a pause keeps it registered so resuming is instant and
    // nothing that arrives in the gap is lost.
    if mount.state.paused {
        return false;
    }
    if mount.state.status.as_deref() == Some("auth_required") {
        return false;
    }
    // The PROVIDER told us to wait. Checked above every other "due" reason —
    // including the backfill fast path below, which is the one that would
    // otherwise ignore it: a throttled mount mid-import runs its chunks
    // back-to-back, so without this guard a 429 with `Retry-After: 300` is
    // answered by another call within seconds. That is the self-sustaining
    // spiral the 503 classification exists to prevent, re-entered through the
    // scheduler instead of the job retry.
    if let Some(until) = mount.state.retry_after {
        if now < until {
            return false;
        }
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

    // A run that was KILLED never cleared its own status, so recover it here.
    //
    // The job watchdog aborts any sync past `timeout_seconds` by dropping the
    // task at its current await point. `finalize` is an ordinary sequential call
    // and there is no `Drop` guard anywhere in this module, so on a kill it never
    // runs: `status` stays `"syncing"` and nothing else in the system ever
    // rewrites it — not `is_due`, not `preflight`, not the boot-time job restore,
    // which resets orphaned JOBS but never mount state.
    //
    // For a polled mount that only wasted a lease TTL. For a WEBHOOK mount it was
    // terminal: the branch below returns `!has_active_push`, so a mount holding a
    // live subscription is never scheduled, and the only thing that could clear
    // the flag — a sync run — was the one thing that could not be scheduled. The
    // console compounds it by grinding `Sync now` out on the same flag. A
    // production calendar mount sat like that for an hour with its last run
    // reporting OK.
    //
    // THIS MUST STAY ABOVE THE WEBHOOK BRANCH, for the same reason the backfill
    // guard above does: below it, the case that needs recovery most cannot reach
    // it.
    //
    // The threshold is deliberately generous. The watchdog guarantees no run
    // outlives `timeout_seconds`, so a run that started more than twice the lease
    // TTL ago cannot still be running, whatever the status claims. Being wrong in
    // the other direction is harmless anyway: the run we enqueue takes the
    // per-mount lease, and a genuinely live sync simply answers `locked_elsewhere`.
    if stale_syncing(mount, now) {
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

/// Whether this mount claims to be syncing but cannot possibly be.
///
/// Measured from the open run's `started_at` rather than `last_attempt_at`,
/// because only `finalize` stamps the latter — and `finalize` is exactly what a
/// killed run skips, so on the path this exists to detect it is always stale.
pub(super) fn stale_syncing(mount: &MountConfig, now: i64) -> bool {
    if mount.state.status.as_deref() != Some("syncing") {
        return false;
    }
    let cutoff = 2 * super::SYNC_LEASE_TTL.as_secs() as i64;
    match mount.state.last_run.as_ref() {
        Some(run) => now - run.started_at > cutoff,
        // Status says syncing with no run record at all: nothing can finish it.
        None => true,
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
            resolver_function: None,
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

    /// A stated `Retry-After` outranks every other reason to run — including
    /// the backfill fast path, which is exactly the one that would otherwise
    /// answer a "wait 5 minutes" with another call within seconds, because
    /// import chunks run back-to-back.
    #[test]
    fn a_provider_stated_backoff_outranks_even_a_pending_backfill() {
        let now = 1_000_000;
        let mut m = mount();
        m.state.retry_after = Some(now + 300);
        assert!(!is_due(&m, now), "must not run before the stated wait");

        // The backfill fast path must not smuggle it past the guard.
        m.state.backfill_cursor = Some("resume-here".to_string());
        m.state.consecutive_failures = 0;
        assert!(
            !is_due(&m, now),
            "a pending backfill does not override a throttle"
        );

        // Once the window elapses, normal scheduling resumes.
        assert!(is_due(&m, now + 300));
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
