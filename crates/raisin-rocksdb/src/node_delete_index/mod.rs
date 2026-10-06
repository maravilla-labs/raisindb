//! `cf::NODE_DELETES`: every node delete, as a key — so "was node N deleted
//! in `(R, B]`?" is one seek instead of a walk over N's history.
//!
//! # Why
//!
//! The translation read rule (`translation_read`, `mvcc_read::NodeLifeline`)
//! ends an overlay version stored at `R`, read at bound `B`, when
//!
//! - (a) the node has a delete tombstone at some `Rd` with `R < Rd <= B`, or
//! - (b) the node's newest `NODES` record at or before `R` is a tombstone (a
//!   delete AT `R`, or a version written into a dead generation).
//!
//! Answering that from `NODES` walks every record of the node from `B` down
//! to `R` — one per edit made after the translation, each a full node blob
//! pulled through the block cache. A page edited 100 times after it was
//! translated paid ~15–19 µs per overlay read, on every localized row.
//!
//! # Key
//!
//! `{tenant}\0{repo}\0{branch}\0{ws}\0{node_id}\0{~rev}`, empty value: the
//! same scope and descending-HLC trailer as the `NODES` key of the tombstone
//! it records (node ids are null-free; the revision is the fixed 16-byte
//! tail and is never split on `\0`). A node's deletes run newest first, so
//! the deletes at or below `B` start at ONE seek, and a node that was never
//! deleted answers "no" from that seek alone.
//!
//! # The invariant, and why it is all the reader needs
//!
//! **Completeness**: on a branch whose index is `Ready`
//! ([`state`]), every `NODES` tombstone has its entry. NOT exactness: an entry
//! whose tombstone is gone (history GC dropped it) or was overwritten is
//! STALE, and the reader confirms each entry it relies on with a seek on
//! `NODES` at exactly that revision ([`lifeline`]). So stale entries cost a
//! seek and never change an answer, and nothing ever has to delete one.
//! Restores and re-creates need no entry: a live record above a delete is
//! found by the same confirmation seek that rule (b) makes on `NODES`.
//!
//! Completeness is kept by
//!
//! - **writing in the delete's own batch, at the one funnel**: every node
//!   delete — transaction, repository, cascade, cross-branch prune, merge,
//!   replication apply — goes through `tombstones::add_node_tombstones_with_parent`,
//!   which puts the `NODES` tombstone and this entry side by side
//!   ([`stage_delete`]). The one tombstone written outside it — a merge's
//!   resolved deletion with no live version on either side
//!   (`repositories::branches::merge::deletion`) — stages its entry too. It
//!   is derived and written locally, so a replica writes it for a REMOTE
//!   delete too (nothing indexed replicates);
//! - **copying it like `NODES`** on a fork (`BRANCH_CF_REGISTRY`, after NODES);
//! - **a backfill** ([`backfill`]) for tombstones written before this index
//!   existed, or brought in by a checkpoint ingest or a branch copy from a
//!   source that was not ready. Until a branch's backfill has completed under
//!   an unchanged generation the branch is not `Ready`, and the reader takes
//!   the old walk — fail-correct, never fail-fast.

pub mod auto;
pub(crate) mod backfill;
mod branch_copy;
pub(crate) mod lifeline;
pub mod state;
#[cfg(test)]
mod tests;

pub use backfill::NodeDeleteCounts;
pub(crate) use lifeline::IndexedLifeline;
pub use state::is_ready;

use crate::{cf, cf_handle, keys::KeyBuilder};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{WriteBatch, DB};

/// `{tenant}\0{repo}\0{branch}\0{ws}\0{node_id}\0` — every delete of one node.
pub fn node_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(node_id)
        .build_prefix()
}

/// The entry recording a delete of `node_id` at `revision`.
pub fn entry_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    revision: &HLC,
) -> Vec<u8> {
    let mut key = node_prefix(tenant_id, repo_id, branch, workspace, node_id);
    key.extend_from_slice(&revision.encode_descending());
    key
}

/// Stage the entry for a delete of `node_id` at `revision` into the batch
/// that writes the delete's `NODES` tombstone. Called by the delete funnel
/// (`tombstones::add_node_tombstones_with_parent`) and by the merge's direct
/// tombstone (`merge::deletion`) — every writer of a `NODES` tombstone.
pub(crate) fn stage_delete(
    batch: &mut WriteBatch,
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
    revision: &HLC,
) -> Result<()> {
    let (tenant_id, repo_id, branch, workspace) = scope;
    batch.put_cf(
        cf_handle(db, cf::NODE_DELETES)?,
        entry_key(tenant_id, repo_id, branch, workspace, node_id, revision),
        b"",
    );
    Ok(())
}

/// Every delete of `node_id` recorded in the index, newest first (tests and
/// diagnostics; readers go through [`lifeline`]).
pub fn recorded_deletes(
    db: &DB,
    scope: (&str, &str, &str, &str),
    node_id: &str,
) -> Result<Vec<HLC>> {
    let (tenant_id, repo_id, branch, workspace) = scope;
    let prefix = node_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let mut out = Vec::new();
    for item in crate::prefix_scan(db, cf_handle(db, cf::NODE_DELETES)?, prefix.clone()) {
        let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        if !key.starts_with(&prefix) {
            break;
        }
        if key.len() == prefix.len() + 16 {
            if let Ok(at) = crate::keys::extract_revision_from_key(&key) {
                out.push(at);
            }
        }
    }
    Ok(out)
}
