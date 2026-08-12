//! Recovering a deleted node's identity from its MVCC pre-image.
//!
//! Split out from the walk itself because this is the one piece of it that is
//! about MVCC rather than about scheduling: a delete's evidence lives at a
//! DIFFERENT revision from the commit that reports it, and getting that offset
//! wrong reads the tombstone instead of the node.

use std::sync::Arc;

use chrono::Utc;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::Storage;

use super::super::config::{MountConfig, PendingDelete};
use super::super::materializer::{node_external_id, node_mount_id, under};
use crate::RocksDBStorage;

/// Recover a deleted node's provenance from the MVCC pre-image.
///
/// The pre-image is read at the revision immediately BEFORE the delete: at the
/// delete's own revision the node is a tombstone and reads as absent, which is
/// precisely the invisibility this walk exists to solve.
///
/// Returns `None` unless the pre-image proves the node was this mount's —
/// `__mount_id` matching, a non-empty `__external_id`, AND a path inside the
/// mount subtree, the same invariant `SyncIndex.by_path` enforces on the read
/// side. Anything less and a user's own node under the mount path could be
/// pushed as a provider delete.
pub(super) async fn recover_delete(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    mount: &MountConfig,
    change: &raisin_storage::NodeChangeInfo,
    revision: &HLC,
    bulk: bool,
) -> Result<Option<PendingDelete>> {
    let Some(before) = predecessor(revision) else {
        return Ok(None);
    };
    let node = storage
        .nodes()
        .get_at_revision(
            tenant,
            repo,
            &mount.target_branch,
            &change.workspace,
            &change.node_id,
            &before,
        )
        .await?;
    let Some(node) = node else {
        // No pre-image: either the node was created and deleted inside the
        // watermark gap (nothing was ever pushed, so there is nothing to
        // retract) or history has been collected. Both are correctly silent.
        return Ok(None);
    };

    if node_mount_id(&node) != Some(mount.mount_id.as_str()) {
        return Ok(None);
    }
    let Some(external_id) = node_external_id(&node).filter(|e| !e.is_empty()) else {
        return Ok(None);
    };

    // OWNERSHIP NEEDS POSITIVE EVIDENCE, and `__mount_id` alone is not it.
    //
    // The reserved properties travel WITH a node: move one out of the mount
    // subtree and it keeps `__mount_id` and `__external_id` indefinitely.
    // `relocate` is what interprets a move — under `move_policy: detach` it
    // strips the node's mount ownership — but that runs on the next drain, and
    // until it does the node still reads as mount-owned here.
    //
    // So: move a node out of the mount, then delete it at its new location
    // before a reconcile pass reaches it, and this queues a remote DELETE of a
    // message, file or event the user only ever moved. Real content destroyed at
    // the provider, by a sequence someone can perform in five seconds without
    // touching the mount at all.
    //
    // Every other layer that decides what this mount owns — `scan_mount_nodes`,
    // the index, `relocate` itself — is path-scoped. This was the one that was
    // not, and "no contrary evidence" is not ownership.
    if !under(&mount.mount_path, &node.path) {
        tracing::info!(
            mount_id = %mount.mount_id,
            path = %node.path,
            mount_path = %mount.mount_path,
            external_id = %external_id,
            "a deleted node carried this mount's ids but sat OUTSIDE its subtree; \
             it had been moved out, so no remote delete is queued for it"
        );
        return Ok(None);
    }

    Ok(Some(PendingDelete {
        node_id: change.node_id.clone(),
        external_id: external_id.to_string(),
        path: Some(node.path.clone()).filter(|p| !p.is_empty()),
        // The delete's concurrency base. Nowhere else has it once the node is
        // gone, so it is captured here or not at all.
        etag: super::super::materializer::node_str_prop(&node, "__etag"),
        revision: revision.to_string(),
        detected_at: Utc::now().timestamp(),
        bulk,
    }))
}

/// The HLC immediately below `h`.
///
/// HLC ordering is `(timestamp_ms, counter)` lexicographic, so the predecessor
/// of a counter-zero revision is the previous millisecond at the maximum
/// counter. Any value strictly between the two would do — the read is an
/// at-or-before lookup — but the true predecessor is the one that cannot skip a
/// same-millisecond commit that landed between them.
fn predecessor(h: &HLC) -> Option<HLC> {
    if h.counter > 0 {
        Some(HLC::new(h.timestamp_ms, h.counter - 1))
    } else if h.timestamp_ms > 0 {
        Some(HLC::new(h.timestamp_ms - 1, u64::MAX))
    } else {
        None
    }
}

#[cfg(test)]
mod predecessor_tests {
    use super::*;

    #[test]
    fn steps_back_one_counter_within_a_millisecond() {
        assert_eq!(predecessor(&HLC::new(7, 3)), Some(HLC::new(7, 2)));
    }

    /// The borrow case. Returning `HLC::new(ts - 1, 0)` here would read the
    /// pre-image at the START of the previous millisecond and miss every
    /// same-millisecond commit in it — which is exactly the shape a batched
    /// import produces.
    #[test]
    fn borrows_into_the_previous_millisecond_at_the_top_counter() {
        assert_eq!(predecessor(&HLC::new(7, 0)), Some(HLC::new(6, u64::MAX)));
    }

    #[test]
    fn has_no_predecessor_at_zero() {
        assert_eq!(predecessor(&HLC::new(0, 0)), None);
    }
}
