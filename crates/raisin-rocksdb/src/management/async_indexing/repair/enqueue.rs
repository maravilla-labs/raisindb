//! Enqueueing a repair as a job on this node, and the checkpoint-ingest hook.
//!
//! The admin fan-out endpoint calls [`enqueue_index_repair`] on every peer;
//! each node repairs its own data and reports through its state record. A
//! repair is never enqueued at boot — except the `node_path` backfill, which
//! `auto_node_path.rs` queues after the job system starts.

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
    enqueue_index_repair_with(
        storage,
        tenant_id,
        repo_id,
        branch,
        kind,
        dry_run,
        HashMap::new(),
        None,
    )
    .await
}

/// [`enqueue_index_repair`] with job-context `metadata` (the automatic
/// `node_path` chain marks its jobs this way) and a retry cap (`None`: the
/// registry default).
pub(super) async fn enqueue_index_repair_with(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: Option<&str>,
    kind: RepairKind,
    dry_run: bool,
    metadata: HashMap<String, serde_json::Value>,
    max_retries: Option<u32>,
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
        metadata,
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
            max_retries,
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
    // The peer's PROPERTY_INDEX arrived with the checkpoint, written by
    // whatever writer the peer runs: skip-unchanged writes stop here until
    // this node rebuilds (Phase 7; queued below when it rebuilds by itself).
    super::property_state::invalidate_all_rebuilds(
        storage.db(),
        tenant_id,
        repo_id,
        &super::repair_node_id(storage),
    )?;
    let kinds = [
        RepairKind::OrderedChildren,
        RepairKind::PathTombstone,
        RepairKind::NodePath,
    ];
    // The peer's data may need these repairs again: until they have re-run,
    // run-collapse refuses the CFs they correct (plan Phase 9 prerequisite).
    mark_repairs_pending(
        storage.db(),
        tenant_id,
        repo_id,
        &super::repair_node_id(storage),
        &kinds,
    )?;
    for kind in kinds {
        enqueue_index_repair(storage, tenant_id, repo_id, None, kind, false).await?;
    }
    // With skip-unchanged on, the rebuild queues itself (plan Phase 7b): the
    // automatic chain, one branch at a time (a no-op while it runs).
    if super::auto_property_index::auto_rebuild_enabled(storage) {
        super::auto_node_path::start_chain(storage, RepairKind::PropertyIndex).await?;
    }
    // The peer's BLOCK_TRANSLATIONS may hold block overlays of deleted nodes
    // with no `T` (reads end them anyway): the cleanup is owed again, through
    // its automatic chain (plan Phase 11c).
    if super::auto_block_overlays::auto_enabled() {
        mark_repairs_pending(
            storage.db(),
            tenant_id,
            repo_id,
            &super::repair_node_id(storage),
            &[RepairKind::BlockOverlayTombstones],
        )?;
        super::auto_node_path::start_chain(storage, RepairKind::BlockOverlayTombstones).await?;
    }
    Ok(())
}

/// Reset this node's state record of each `kind` on every branch of the
/// repository to `queued` (no cursor): the repair is owed again. A record is
/// a progress record, never what a repair decides its targets from, so this
/// changes no repair's work — it is what run-collapse's prerequisite check
/// (`history_gc::collapse::required_repairs`) and the console read.
pub fn mark_repairs_pending(
    db: &rocksdb::DB,
    tenant_id: &str,
    repo_id: &str,
    node_id: &str,
    kinds: &[RepairKind],
) -> Result<()> {
    for branch in super::branches::list_branches(db, tenant_id, repo_id)? {
        mark_repairs_pending_on(db, tenant_id, repo_id, &branch, node_id, kinds)?;
    }
    Ok(())
}

/// [`mark_repairs_pending`] on one branch.
pub fn mark_repairs_pending_on(
    db: &rocksdb::DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    kinds: &[RepairKind],
) -> Result<()> {
    let cf = crate::cf_handle(db, crate::cf::INDEX_STATUS)?;
    let mut batch = rocksdb::WriteBatch::default();
    for kind in kinds {
        let state = super::RepairState {
            status: "queued".to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            ..super::RepairState::default()
        };
        let bytes = serde_json::to_vec(&state)
            .map_err(|e| raisin_error::Error::storage(format!("repair state encode: {e}")))?;
        batch.put_cf(
            cf,
            super::state_key(tenant_id, repo_id, branch, kind.slug(), node_id),
            bytes,
        );
    }
    db.write(batch)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))
}

/// The checkpoint-ingest hook: re-enqueue every repair for every repository
/// the database now holds. Called after `invalidate_derived_caches_for_database`.
/// Returns the number of repositories enqueued for.
pub async fn reenqueue_repairs_after_checkpoint(storage: &RocksDBStorage) -> Result<usize> {
    let repositories = list_repositories(storage.db())?;
    for (tenant_id, repo_id) in &repositories {
        reenqueue_repairs_after_ingest(storage, tenant_id, repo_id).await?;
    }
    Ok(repositories.len())
}

/// Every `(tenant, repo)` with at least one branch record.
pub(crate) fn list_repositories(db: &rocksdb::DB) -> Result<Vec<(String, String)>> {
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
