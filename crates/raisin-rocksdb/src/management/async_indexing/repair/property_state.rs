//! The per-node PROPERTY_INDEX rebuild state that gates skip-unchanged
//! writes (plan Phase 7), and what invalidates it.

use super::{enqueue, load_state, state_key, RepairKind, RepairReport};
use crate::cf;
use raisin_error::Result;
use rocksdb::DB;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Serializes an invalidation against a rebuild's final "is my epoch still
/// current? then `done`" commit, so an invalidation can never land between
/// the check and the write.
static GATE: Mutex<()> = Mutex::new(());

pub(super) fn gate_lock() -> MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether `branch`'s PROPERTY_INDEX has been rebuilt to completion by the
/// Phase 7 rebuild ON THIS NODE (`node_id`), the precondition for
/// `index.skip_unchanged`. A record under another node's id (a peer's,
/// arriving in a checkpoint) never counts.
pub fn property_index_rebuilt(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
) -> bool {
    load_state(
        db,
        tenant_id,
        repo_id,
        branch,
        RepairKind::PropertyIndex.slug(),
        node_id,
    )
    .ok()
    .flatten()
    .is_some_and(|state| state.status == "done")
}

/// Mark `branch`'s PROPERTY_INDEX rebuild as no longer valid on this node, so
/// the delta writer does full puts there until it is rebuilt again (a verify
/// miss, a checkpoint ingest, a merge from an unrebuilt source).
///
/// UNCONDITIONAL, whatever the record says: a rebuild that is `running` (or
/// crashed and waiting to resume from its cursor) would otherwise carry on
/// past the nodes the invalidating event changed and commit `done`. So the
/// record goes to `stale` with no cursor (a later run starts from the first
/// node), and the branch's invalidation EPOCH is replaced: a rebuild still in
/// flight in this process overwrites the record with its own progress, but it
/// compares the epoch it started under before committing `done`
/// ([`rebuild_epoch`]) and finishes `stale` instead.
pub fn invalidate_property_index_rebuild(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
) -> Result<()> {
    let slug = RepairKind::PropertyIndex.slug();
    let cf_status = crate::cf_handle(db, cf::INDEX_STATUS)?;
    let _gate = gate_lock();
    let mut batch = rocksdb::WriteBatch::default();
    if let Some(mut state) = load_state(db, tenant_id, repo_id, branch, slug, node_id)? {
        state.status = "stale".to_string();
        state.cursor = None;
        state.updated_at = chrono::Utc::now().to_rfc3339();
        let bytes = serde_json::to_vec(&state)
            .map_err(|e| raisin_error::Error::storage(format!("repair state encode: {e}")))?;
        batch.put_cf(
            cf_status,
            state_key(tenant_id, repo_id, branch, slug, node_id),
            bytes,
        );
    }
    batch.put_cf(
        cf_status,
        epoch_key(tenant_id, repo_id, branch, node_id),
        uuid::Uuid::new_v4().to_string().as_bytes(),
    );
    db.write(batch)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))
}

/// The branch's current invalidation epoch for the `property_index` rebuild
/// on this node (`None` before the first invalidation). A rebuild records it
/// when it starts (in its state record, so a resumed run keeps the original)
/// and commits `done` only if it is unchanged.
pub(super) fn rebuild_epoch(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
) -> Result<Option<String>> {
    let cf_status = crate::cf_handle(db, cf::INDEX_STATUS)?;
    Ok(db
        .get_cf(cf_status, epoch_key(tenant_id, repo_id, branch, node_id))
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
}

/// The final commit of a COMPLETED `property_index` rebuild: `done` when the
/// invalidation epoch it started under ([`RepairState::epoch`], kept across a
/// resume) is still current, else `stale` with no cursor — nodes before the
/// cursor may have changed since they were passed, so the next run starts
/// from the beginning. The check and the write happen under the gate, so an
/// invalidation cannot land between them. Returns whether it committed `done`.
///
/// [`RepairState::epoch`]: super::RepairState::epoch
pub(super) fn commit_rebuild(
    db: &DB,
    writer: &mut super::cursor::BoundedWriter<'_>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
) -> Result<bool> {
    let _gate = gate_lock();
    if writer.state().epoch == rebuild_epoch(db, tenant_id, repo_id, branch, node_id)? {
        writer.commit("done")?;
        return Ok(true);
    }
    tracing::warn!(
        tenant_id,
        repo_id,
        branch,
        "property index rebuild was invalidated while running; not marking it done"
    );
    writer.clear_cursor();
    writer.commit("stale")?;
    Ok(false)
}

/// `{tenant}\0{repo}\0{branch}\0repair_epoch\0property_index\0{node_id}`
/// — beside the state record, under the same branch prefix (so the branch
/// cleanup in [`forget_branch_rebuild_state`] reaches both).
fn epoch_key(tenant_id: &str, repo_id: &str, branch: &str, node_id: &str) -> Vec<u8> {
    crate::keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("repair_epoch")
        .push(RepairKind::PropertyIndex.slug())
        .push(node_id)
        .build()
}

/// Drop every repair state record and epoch of `branch` (all repairs, all
/// node ids), staged into `batch`. A branch deleted and later re-created under
/// the same name must not inherit a `done`: the new branch's index is a copy
/// of whatever it was forked from, never rebuilt here.
pub fn forget_branch_rebuild_state(
    db: &DB,
    batch: &mut rocksdb::WriteBatch,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<()> {
    // A branch created or deleted under this name must not inherit the cached
    // index definitions of the one before it either (`indexing::compound::defs`).
    crate::indexing::compound::defs::invalidate_branch(db, tenant_id, repo_id, branch);
    let cf_status = crate::cf_handle(db, cf::INDEX_STATUS)?;
    for marker in ["repair_state", "repair_epoch"] {
        let prefix = crate::keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(marker)
            .build_prefix();
        if let Some(end) = crate::prefix_successor(&prefix) {
            batch.delete_range_cf(cf_status, prefix, end);
        }
    }
    // The localized name index's per-workspace build records too: a branch
    // re-created under this name is a fork, never built here, and must not
    // read the deleted branch's `Ready`.
    crate::localized_name::state::stage_forget_branch(db, batch, tenant_id, repo_id, branch)?;
    Ok(())
}

/// After a verify: every branch it found a miss on gets the rebuild queued on
/// this node (its state was already reset by the run, so writes there are
/// full puts until the rebuild completes).
pub(super) async fn queue_rebuilds_for_misses(
    storage: &crate::RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    reports: &[RepairReport],
) -> Result<()> {
    for report in reports {
        if report.completed && report.property_index.missing > 0 {
            tracing::warn!(
                tenant_id,
                repo_id,
                branch = %report.branch,
                missing = report.property_index.missing,
                "property index verify found missing entries; queueing a rebuild"
            );
            enqueue::enqueue_index_repair(
                storage,
                tenant_id,
                repo_id,
                Some(&report.branch),
                RepairKind::PropertyIndex,
                false,
            )
            .await?;
        }
    }
    Ok(())
}

/// After a checkpoint ingest: the peer's PROPERTY_INDEX was merged in, and
/// nothing says its writer was this one — every branch of the repository goes
/// back to full puts on this node until it is rebuilt (admin-triggered).
pub(super) fn invalidate_all_rebuilds(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    node_id: &str,
) -> Result<()> {
    for branch in super::branches::list_branches(db, tenant_id, repo_id)? {
        invalidate_property_index_rebuild(db, tenant_id, repo_id, &branch, node_id)?;
    }
    Ok(())
}

/// [`invalidate_all_rebuilds`] for every repository the database holds. The
/// checkpoint ingest calls it BEFORE the copy starts (local writes keep
/// running during the copy, and must not skip against the peer versions it
/// brings in) and again AFTER it (the copy may have brought this node's own
/// record back from a peer that once ingested it). Returns the repositories
/// invalidated.
pub fn invalidate_rebuilds_for_ingest(storage: &crate::RocksDBStorage) -> Result<usize> {
    let node_id = super::repair_node_id(storage);
    let repositories = super::enqueue::list_repositories(storage.db())?;
    for (tenant_id, repo_id) in &repositories {
        invalidate_all_rebuilds(storage.db(), tenant_id, repo_id, &node_id)?;
    }
    Ok(repositories.len())
}

/// After `copy_branch_indexes` replayed `source`'s entries into `target`
/// (merge, fork): invalidate `target`'s rebuild for every node id that holds
/// a `property_index` record there, unless the same node id has also rebuilt
/// `source` (then the replayed entries are rebuilt entries).
pub fn invalidate_after_branch_copy(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    source: &str,
    target: &str,
) -> Result<()> {
    let slug = RepairKind::PropertyIndex.slug();
    let prefix = crate::keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(target)
        .push("repair_state")
        .push(slug)
        .build_prefix();
    let cf_status = crate::cf_handle(db, cf::INDEX_STATUS)?;
    let mut node_ids = Vec::new();
    for item in crate::prefix_scan(db, cf_status, prefix.clone()) {
        let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        if !key.starts_with(&prefix) {
            break;
        }
        node_ids.push(String::from_utf8_lossy(&key[prefix.len()..]).into_owned());
    }
    for node_id in node_ids {
        if !property_index_rebuilt(db, tenant_id, repo_id, source, &node_id) {
            invalidate_property_index_rebuild(db, tenant_id, repo_id, target, &node_id)?;
        }
    }
    Ok(())
}
