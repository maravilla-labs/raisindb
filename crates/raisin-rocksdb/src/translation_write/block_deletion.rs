//! A node delete's block-overlay tombstones, materialized (plan Phase 11c).
//!
//! The read rule (`translation_read::ended_by_node_delete`) is what makes a
//! deleted node's block overlays absent, in any arrival order. These writes
//! make the STORED history say the same — `T` at the delete's revision — so
//! retention GC reclaims the versions below it (it keeps the newest version
//! at or below its cutoff, which would otherwise be the live one, forever)
//! and a raw scan of `BLOCK_TRANSLATIONS` sees the deletion. A `T` at a
//! delete's revision `Rd` can never disagree with the rule: every version at
//! or below `Rd` is ended for reads at or after `Rd`, and versions above it
//! are untouched. So a materialization that loses a race (a version committed
//! below a delete after the delete read the CF, two peers' batches applied
//! concurrently, a checkpoint ingest) changes no answer. Its storage is NOT
//! caught up automatically on a branch whose cleanup already reached `done`:
//! the `block_overlay_tombstones` repair re-runs there only after a
//! checkpoint ingest or from the admin endpoint, and otherwise history GC
//! stores the `T` when it drops the delete (`materialize_node_deletion`).
//! Until then the live version is retained, never served.
//!
//! A version written ABOVE a delete into the dead generation it opened is
//! ended by the read rule but never materialized here (see
//! `dead_generation`): only history GC, dropping that delete, stores its `T`.
//!
//! Three writers, one body:
//! - the delete funnel (`tombstones::add_node_tombstones_with_parent`, step
//!   13: transaction, repository, cascade, merge, cross-branch copy and the
//!   replication apply path) and history GC, before it drops a node tombstone
//!   (`materialize_node_deletion`) — [`materialize_block_deletion`];
//! - the one writer, when a LIVE block version lands below a stored delete
//!   (a replicated version arriving after the delete it precedes, or a local
//!   write racing one) — [`stage_late_version`];
//! - the repair — [`block_deletion_keys`].
//!
//! Derived and local: nothing here is captured for replication; every node
//! derives the same `T` from the same versions.

use super::{stage_version, OverlayTarget};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{WriteBatch, DB};

/// One block overlay a delete tombstones.
#[derive(Debug, Clone)]
pub(crate) struct BlockDeletion {
    pub block_uuid: String,
    pub locale: String,
    /// The live version the delete ends.
    pub live_revision: HLC,
    /// The `T` key, at the delete's revision.
    pub key: Vec<u8>,
}

/// Every block overlay of `node_id` whose newest STORED version at or before
/// `deleted_at` is live, with the key of its `T` at `deleted_at`. Empty when
/// that `T` is already stored (it is then the newest version there), so a
/// repeated materialization writes nothing.
pub(crate) fn block_deletion_keys(
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    deleted_at: &HLC,
) -> Result<Vec<BlockDeletion>> {
    let (tenant_id, repo_id, branch, workspace) = scope;
    let live =
        crate::translation_read::stored_live_block_overlays(db, scope, node_id, Some(deleted_at))?;
    Ok(live
        .into_iter()
        .map(|(block_uuid, locale, live_revision)| {
            let key = OverlayTarget {
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_id,
                block_uuid: Some(&block_uuid),
                locale: &locale,
            }
            .data_key(deleted_at);
            BlockDeletion {
                block_uuid,
                locale,
                live_revision,
                key,
            }
        })
        .collect())
}

/// Stage `T` at `deleted_at` for every block overlay of `node_id` live
/// there ([`block_deletion_keys`]). Returns how many were staged.
pub(crate) fn materialize_block_deletion(
    db: &DB,
    batch: &mut WriteBatch,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    deleted_at: &HLC,
) -> Result<usize> {
    let (tenant_id, repo_id, branch, workspace) = scope;
    let deletions = block_deletion_keys(db, scope, node_id, deleted_at)?;
    for deletion in &deletions {
        let target = OverlayTarget {
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            block_uuid: Some(&deletion.block_uuid),
            locale: &deletion.locale,
        };
        stage_version(db, batch, &target, None, deleted_at)?;
    }
    Ok(deletions.len())
}

/// A live block version of `target` is being staged at `revision`: when the
/// node has a stored delete at or above it, stage `T` at the OLDEST such
/// delete — the revision from which the read rule ends this version — so the
/// result is the same whether the delete or the version was stored first.
pub(super) fn stage_late_version(
    db: &DB,
    batch: &mut WriteBatch,
    target: &OverlayTarget<'_>,
    revision: &HLC,
) -> Result<()> {
    let scope = (
        target.tenant_id,
        target.repo_id,
        target.branch,
        target.workspace,
    );
    let deletes = crate::mvcc_read::deletes_in_range(db, scope, target.node_id, revision, None)?;
    if let Some(first) = deletes.last() {
        stage_version(db, batch, target, None, first)?;
    }
    Ok(())
}
