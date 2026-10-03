//! Enqueueing a repair as a job on this node, and the checkpoint-ingest hook.
//!
//! The admin fan-out endpoint calls [`enqueue_index_repair`] on every peer;
//! each node repairs its own data and reports through its state record. A
//! repair is never enqueued at boot.

use super::RepairKind;
use crate::RocksDBStorage;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::jobs::{JobContext, JobId, JobType};
use std::collections::HashMap;

/// Register a `JobType::IndexRepair` on this node. Returns the job id.
pub async fn enqueue_index_repair(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: Option<&str>,
    kind: RepairKind,
    dry_run: bool,
) -> Result<String> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let context = JobContext {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch.unwrap_or_default().to_string(),
        workspace_id: String::new(),
        revision: HLC::new(now_ms, 0),
        metadata: HashMap::new(),
    };

    // Context before registration, so dispatch never sees a job without one.
    let job_id = JobId::new();
    storage.job_data_store().put(&job_id, &context)?;
    storage
        .job_registry()
        .register_job_with_id(
            job_id.clone(),
            JobType::IndexRepair {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                branch: branch.map(str::to_string),
                repair: kind.slug().to_string(),
                dry_run,
            },
            tenant_id.to_string(),
            None,
            None,
            None,
        )
        .await?;
    tracing::info!(
        job_id = %job_id,
        tenant_id,
        repo_id,
        branch = branch.unwrap_or("*"),
        repair = kind.slug(),
        dry_run,
        "Queued index repair job"
    );
    Ok(job_id.to_string())
}

/// Re-enqueue every repair for `(tenant, repo)` after a checkpoint ingest.
///
/// A checkpoint is a put-merge of every CF from the peer, so an ingest from an
/// unrepaired (or older) peer brings the damage straight back. The repairs are
/// data-detected, so re-running them over already-clean data writes nothing.
pub async fn reenqueue_repairs_after_ingest(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<()> {
    for kind in [RepairKind::OrderedChildren, RepairKind::PathTombstone] {
        enqueue_index_repair(storage, tenant_id, repo_id, None, kind, false).await?;
    }
    Ok(())
}

/// The checkpoint-ingest hook: re-enqueue every repair for every repository
/// the database now holds. Called next to `invalidate_all_derived_caches`.
/// Returns the number of repositories enqueued for.
pub async fn reenqueue_repairs_after_checkpoint(storage: &RocksDBStorage) -> Result<usize> {
    let repositories = list_repositories(storage.db())?;
    for (tenant_id, repo_id) in &repositories {
        reenqueue_repairs_after_ingest(storage, tenant_id, repo_id).await?;
    }
    Ok(repositories.len())
}

/// Every `(tenant, repo)` with at least one branch record.
fn list_repositories(db: &rocksdb::DB) -> Result<Vec<(String, String)>> {
    let cf = crate::cf_handle(db, crate::cf::BRANCHES)?;
    let mut iter = db.raw_iterator_cf(cf);
    iter.seek_to_first();
    let mut out: Vec<(String, String)> = Vec::new();
    while iter.valid() {
        if let Some(key) = iter.key() {
            // {tenant}\0{repo}\0branches\0{branch}
            let mut parts = key.splitn(4, |b| *b == 0);
            if let (Some(t), Some(r), Some(b"branches")) =
                (parts.next(), parts.next(), parts.next())
            {
                let pair = (
                    String::from_utf8_lossy(t).into_owned(),
                    String::from_utf8_lossy(r).into_owned(),
                );
                if out.last() != Some(&pair) {
                    out.push(pair);
                }
            }
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    out.dedup();
    Ok(out)
}
