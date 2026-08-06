//! The periodic driver: find masters, expand them, converge the projection.
//!
//! Discovery is deliberately NOT mount-driven. A recurring event is a
//! `raisin:Event` with a rule, whoever wrote it — a calendar mount today, a
//! local `mirror` create once Stage 7 lands — and keying the projection off a
//! mount would silently exclude the second. The unit of work is therefore a
//! WORKSPACE, and a workspace with no series masters costs one type-index scan
//! and stops.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use raisin_error::Result;
use raisin_locks::LockManagerHandle;
use raisin_storage::{RepoScope, RepositoryManagementRepository, Storage, WorkspaceRepository};

use super::rebuild::rebuild_locked;
use super::REBUILD_INTERVAL_SECS;
use crate::RocksDBStorage;

/// What one rebuild pass did. Counts, not nodes — the caller logs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebuildSummary {
    pub workspaces_scanned: usize,
    pub masters: usize,
    pub written: usize,
    pub pruned: usize,
    pub unchanged: usize,
    pub suppressed: usize,
    pub failed_masters: usize,
    pub truncated_masters: usize,
    /// Workspaces skipped because another node (or this one, recently) holds
    /// the rebuild lease.
    pub skipped_leased: usize,
}

impl RebuildSummary {
    fn merge(&mut self, other: RebuildSummary) {
        self.workspaces_scanned += other.workspaces_scanned;
        self.masters += other.masters;
        self.written += other.written;
        self.pruned += other.pruned;
        self.unchanged += other.unchanged;
        self.suppressed += other.suppressed;
        self.failed_masters += other.failed_masters;
        self.truncated_masters += other.truncated_masters;
        self.skipped_leased += other.skipped_leased;
    }

    fn touched(&self) -> bool {
        self.written > 0 || self.pruned > 0
    }
}

/// Process-local "when did I last rebuild this workspace" map.
///
/// The first of the two throttles. It is NOT sufficient on its own — job dedup
/// in this engine is an in-memory `HashMap`, so every node of a cluster runs its
/// own copy of this job and would each pass their own throttle. The lease below
/// is what makes it once-per-cluster.
fn last_run() -> &'static Mutex<HashMap<String, Instant>> {
    static LAST_RUN: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    LAST_RUN.get_or_init(|| Mutex::new(HashMap::new()))
}

fn throttled(key: &str) -> bool {
    let mut map = last_run().lock().unwrap_or_else(|e| e.into_inner());
    let interval = Duration::from_secs(REBUILD_INTERVAL_SECS);
    match map.get(key) {
        Some(at) if at.elapsed() < interval => true,
        _ => {
            map.insert(key.to_string(), Instant::now());
            false
        }
    }
}

fn lease_key(tenant: &str, repo: &str, branch: &str, workspace: &str) -> String {
    format!("{tenant}\0{repo}\0{branch}\0calendar-expand:{workspace}")
}

/// Rebuild the occurrence projection everywhere in scope.
pub async fn run_occurrence_rebuild(
    storage: &Arc<RocksDBStorage>,
    lock_manager: Option<&LockManagerHandle>,
    instance_id: &str,
    tenant_filter: Option<String>,
    repo_filter: Option<String>,
) -> Result<RebuildSummary> {
    let mut summary = RebuildSummary::default();
    for (tenant, repo) in scan_scope(storage, tenant_filter, repo_filter).await? {
        match rebuild_repo(storage, lock_manager, instance_id, &tenant, &repo).await {
            Ok(s) => summary.merge(s),
            Err(e) => tracing::warn!(
                tenant = %tenant, repo = %repo, error = %e,
                "calendar-expand: repo scan failed"
            ),
        }
    }
    Ok(summary)
}

/// Tenants × repos in scope. Mirrors the virtual-mount check's own scan; kept
/// local rather than widening that module's visibility, since the two walk the
/// same registry for unrelated reasons.
async fn scan_scope(
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
            None => match crate::management::list_repositories(storage, &tenant).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, error = %e, "calendar-expand: list repos failed");
                    continue;
                }
            },
        };
        out.extend(repos.into_iter().map(|repo| (tenant.clone(), repo)));
    }
    Ok(out)
}

async fn rebuild_repo(
    storage: &Arc<RocksDBStorage>,
    lock_manager: Option<&LockManagerHandle>,
    instance_id: &str,
    tenant: &str,
    repo: &str,
) -> Result<RebuildSummary> {
    let branch = storage
        .repository_management()
        .get_repository(tenant, repo)
        .await
        .ok()
        .flatten()
        .map(|r| r.config.default_branch)
        .unwrap_or_else(|| "main".to_string());

    let workspaces = storage
        .workspaces()
        .list(RepoScope::new(tenant, repo))
        .await?;
    let mut summary = RebuildSummary::default();
    for ws in workspaces {
        match rebuild_workspace(
            storage,
            lock_manager,
            instance_id,
            tenant,
            repo,
            &branch,
            &ws.name,
        )
        .await
        {
            Ok(s) => summary.merge(s),
            Err(e) => tracing::warn!(
                tenant = %tenant, repo = %repo, workspace = %ws.name, error = %e,
                "calendar-expand: workspace rebuild failed"
            ),
        }
    }
    Ok(summary)
}

/// Rebuild one workspace, under both throttles.
#[allow(clippy::too_many_arguments)]
pub(super) async fn rebuild_workspace(
    storage: &Arc<RocksDBStorage>,
    lock_manager: Option<&LockManagerHandle>,
    instance_id: &str,
    tenant: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
) -> Result<RebuildSummary> {
    let key = lease_key(tenant, repo, branch, workspace);
    if throttled(&key) {
        return Ok(RebuildSummary::default());
    }
    // The lease is acquired for the full rebuild interval and DELIBERATELY NOT
    // released on success: holding it IS the cluster-wide throttle. A crashed
    // node's lease expires on its own, so the worst case is one skipped
    // interval, and the projection is idempotent so a skipped interval costs
    // nothing but freshness.
    if let Some(lm) = lock_manager {
        let owner = format!("{instance_id}:calendar-expand");
        let ttl = Duration::from_secs(REBUILD_INTERVAL_SECS);
        if lm.try_acquire(&key, &owner, ttl).await?.is_none() {
            return Ok(RebuildSummary {
                skipped_leased: 1,
                ..Default::default()
            });
        }
    }
    let summary = rebuild_locked(storage, tenant, repo, branch, workspace).await?;
    if summary.touched() {
        tracing::info!(
            tenant = %tenant, repo = %repo, workspace = %workspace,
            masters = summary.masters,
            written = summary.written,
            pruned = summary.pruned,
            unchanged = summary.unchanged,
            suppressed = summary.suppressed,
            failed = summary.failed_masters,
            truncated = summary.truncated_masters,
            "calendar occurrence projection rebuilt"
        );
    }
    Ok(summary)
}
