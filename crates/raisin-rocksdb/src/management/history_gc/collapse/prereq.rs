//! Which repairs must have completed, per `(branch, CF)`, before collapse may
//! touch that CF there.
//!
//! A repair that writes a tombstone at a HISTORICAL revision is an insert
//! below existing entries: collapse on the unrepaired data can delete a
//! version the repair's tombstone would have separated from its twin, and the
//! repair then hides it (the plan's "deleted at rdel with a missing tombstone,
//! re-created at r3" case). So a CF a repair corrects is collapsed only after
//! that repair's per-node state record says `done` on this branch.
//!
//! - **ORDERED_CHILDREN**: the `ordered_children` repair (Phase 2.1), and the
//!   `node_path` backfill (Phase 10), which the repair's parent resolution
//!   reads through.
//! - **PROPERTY_INDEX, REFERENCE_INDEX, UNIQUE_INDEX, COMPOUND_INDEX**: no
//!   Phase 2 repair writes them. Their rebuilds (Phase 7's `property_index`,
//!   the REFERENCE and COMPOUND rebuilds) either only PUT states that are true
//!   at their version's revision — which collapse never contradicts — or clear
//!   and re-derive the keyspace; both run under the `(branch, CF)` exclusion.
//!
//! The checkpoint-ingest hook resets these records
//! (`repair::reenqueue_repairs_after_ingest`), so data imported from an
//! unrepaired peer re-arms the refusal until the repairs ran again. A branch
//! copy does the same on one branch ([`rearm_after_branch_copy`]): a MERGE
//! replays the source's history into a target whose records may say `done`.

use crate::cf;
use crate::management::async_indexing::repair::{load_state, mark_repairs_pending_on, RepairKind};
use raisin_error::Result;
use rocksdb::DB;

/// The repairs that must be `done` on a branch before `cf_name` is collapsed.
pub fn required_repairs(cf_name: &str) -> &'static [RepairKind] {
    match cf_name {
        cf::ORDERED_CHILDREN => &[RepairKind::OrderedChildren, RepairKind::NodePath],
        _ => &[],
    }
}

/// The slugs of the required repairs whose state record on this node is not
/// `done` (missing, running, queued after an ingest, or refused).
pub(super) fn pending_repairs(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    cf_name: &str,
) -> Result<Vec<String>> {
    let mut pending = Vec::new();
    for kind in required_repairs(cf_name) {
        let state = load_state(db, tenant_id, repo_id, branch, kind.slug(), node_id)?;
        if state.is_none_or(|s| s.status != "done") {
            pending.push(kind.slug().to_string());
        }
    }
    Ok(pending)
}

/// After a branch copy (fork or merge) replayed `source`'s history into
/// `target` at its original revisions: every prerequisite repair whose record
/// on `target` says `done` while `source`'s record for the same node does not
/// is reset to `queued` on `target` — the target now holds history that repair
/// never saw, so collapse must refuse there until it ran again. Mirrors
/// `repair::invalidate_after_branch_copy` (the PROPERTY_INDEX rebuild state).
pub fn rearm_after_branch_copy(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    source: &str,
    target: &str,
) -> Result<()> {
    let mut kinds: Vec<RepairKind> = Vec::new();
    for cf_name in super::COLLAPSE_CFS {
        for kind in required_repairs(cf_name) {
            if !kinds.contains(kind) {
                kinds.push(*kind);
            }
        }
    }
    let cf_status = crate::cf_handle(db, cf::INDEX_STATUS)?;
    for kind in kinds {
        let prefix = crate::keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(target)
            .push("repair_state")
            .push(kind.slug())
            .build_prefix();
        let mut node_ids = Vec::new();
        for item in crate::prefix_scan(db, cf_status, prefix.clone()) {
            let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            if !key.starts_with(&prefix) {
                break;
            }
            node_ids.push(String::from_utf8_lossy(&key[prefix.len()..]).into_owned());
        }
        for node_id in node_ids {
            let done = |branch: &str| -> Result<bool> {
                Ok(
                    load_state(db, tenant_id, repo_id, branch, kind.slug(), &node_id)?
                        .is_some_and(|s| s.status == "done"),
                )
            };
            if done(target)? && !done(source)? {
                mark_repairs_pending_on(db, tenant_id, repo_id, target, &node_id, &[kind])?;
            }
        }
    }
    Ok(())
}
