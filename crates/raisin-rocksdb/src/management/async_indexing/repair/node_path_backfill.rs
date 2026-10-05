//! Phase 10 backfill: give every legacy full-`Node` blob revision the
//! `NODE_PATH` entry its writer never wrote.
//!
//! Before Phase 10 the transaction write path stored the full `Node` (path
//! embedded) and no `NODE_PATH` entry. The read rule (`mvcc_read::
//! materialize_path`: the newer of `NODE_PATH` and the embedded path) already
//! reads such nodes correctly, permanently — so this is cleanup, not the fix.
//! What it buys is a `NODE_PATH` that answers correctly ON ITS OWN, at every
//! revision, which is what lets readers stop decoding the blob for its path.
//!
//! **Every divergent revision, not only the newest.** For each full-blob
//! version at revision R whose embedded path differs from `NODE_PATH` newest
//! ≤ R (or has no live entry there), write `NODE_PATH` at R. Backfilling only
//! nodes with no entry at all, at their latest revision, misses exactly the
//! affected population — a repository-created node renamed through the old
//! `put_node` HAS an entry, just a stale one — and leaves time travel to the
//! rename revision wrong forever.
//!
//! Data-detected and idempotent: a version whose path `NODE_PATH` already
//! answers writes nothing, so a re-run (after a crash or a checkpoint ingest
//! from a peer that still runs the old writer) only fills what is missing.
//!
//! **Never over an entry at the blob's OWN revision.** An entry at exactly R
//! that names another path is a second writer at R — a move in the same
//! transaction, or an in-place `versionable=false` write — and the read rule
//! settles that tie through `PATH_INDEX`, not by trusting the blob. Writing
//! the blob's path over it destroyed the only record of a moved path, so
//! such a version is counted in `conflicts`, logged, and left alone. And
//! because an entry is decided long before its batch commits, it is not
//! queued when decided: `node_path_stage.rs` holds it and re-checks the blob
//! and the slot under the in-place guard right before the write, so a node
//! rewritten in place meanwhile is skipped, not reverted.

use super::super::node_key_parse::parse_node_key;
use super::cursor::BoundedWriter;
use super::node_path_stage::Stage;
use crate::mvcc_read::{embedded_path_of, newest_at_or_before};
use crate::repositories::nodes::helpers::is_tombstone;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{ReadOptions, DB};
use serde::{Deserialize, Serialize};

pub(super) const PASS_NODE_PATH: &str = "node_path";

/// What the backfill found on one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodePathCounts {
    /// Node versions (blob revisions, tombstones excluded) read.
    pub versions: u64,
    /// Of those, legacy full-`Node` blobs carrying an embedded path.
    pub legacy_versions: u64,
    /// `NODE_PATH` entries written (or, in a dry run, that would be).
    pub written: u64,
    /// Legacy versions skipped because `NODE_PATH` already holds a DIFFERENT
    /// entry at the blob's own revision (see the module docs). Reads still
    /// answer them through the read rule's tie-break.
    #[serde(default)]
    pub conflicts: u64,
    /// Legacy versions skipped because the blob was rewritten in place while
    /// the scan ran.
    #[serde(default)]
    pub changed_during_scan: u64,
}

/// The branch's scope, for building `NODE_PATH` keys.
pub(super) struct Scope<'a> {
    pub(super) tenant_id: &'a str,
    pub(super) repo_id: &'a str,
    pub(super) branch: &'a str,
}

/// One node's versions, gathered while the scan is inside its key group.
struct Group {
    /// `{branch prefix}{ws}\0nodes\0{id}\0` — everything but the revision.
    prefix: Vec<u8>,
    workspace: String,
    node_id: String,
    /// `(revision, embedded path)` of every legacy full-blob version.
    legacy: Vec<(HLC, String)>,
    /// The last key of the group, which the cursor records.
    last_key: Vec<u8>,
}

/// Stream the branch's `NODES` and fill `NODE_PATH`. Returns `false` when the
/// run must stop early (resumable from the persisted cursor).
pub(super) fn node_path_pass(
    db: &DB,
    scope: &Scope<'_>,
    writer: &mut BoundedWriter<'_>,
    counts: &mut NodePathCounts,
) -> Result<bool> {
    writer.begin_pass(PASS_NODE_PATH);
    let branch_prefix = keys::branch_prefix(scope.tenant_id, scope.repo_id, scope.branch);
    let cursor = writer
        .state()
        .cursor
        .as_deref()
        .and_then(|h| hex::decode(h).ok());

    let cf_nodes = cf_handle(db, cf::NODES)?;
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    opts.fill_cache(false);
    if let Some(upper) = crate::prefix_successor(&branch_prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf_nodes, opts);
    match cursor {
        Some(mut after) => {
            after.push(0);
            iter.seek(&after);
        }
        None => iter.seek(&branch_prefix),
    }

    let mut group: Option<Group> = None;
    let mut stage = Stage::default();
    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        let Some((workspace, node_id, revision)) = parse_node_key(&branch_prefix, key) else {
            iter.next();
            continue;
        };
        let prefix = &key[..key.len() - 16];
        if group.as_ref().is_some_and(|g| g.prefix != prefix) {
            let done = group.take().expect("checked above");
            if !finish_group(db, scope, writer, &mut stage, counts, done)? {
                return Ok(false);
            }
        }
        let current = group.get_or_insert_with(|| Group {
            prefix: prefix.to_vec(),
            workspace: workspace.to_string(),
            node_id: node_id.to_string(),
            legacy: Vec::new(),
            last_key: Vec::new(),
        });
        current.last_key = key.to_vec();
        if !is_tombstone(value) {
            counts.versions += 1;
            if let Some(path) = embedded_path_of(value) {
                counts.legacy_versions += 1;
                current.legacy.push((revision, path));
            }
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    if let Some(done) = group {
        if !finish_group(db, scope, writer, &mut stage, counts, done)? {
            return Ok(false);
        }
    }
    // What is still staged commits now: the caller's final commit (`done`)
    // writes the batch without the re-check.
    if !stage.is_empty() && !stage.commit(db, scope, writer, counts)? {
        return Ok(false);
    }
    Ok(true)
}

/// Stage the entries one node's legacy versions need, oldest first, then
/// checkpoint past the node (committing when the batch is full).
fn finish_group(
    db: &DB,
    scope: &Scope<'_>,
    writer: &mut BoundedWriter<'_>,
    stage: &mut Stage,
    counts: &mut NodePathCounts,
    mut group: Group,
) -> Result<bool> {
    if !group.legacy.is_empty() {
        let cf_node_path = cf_handle(db, cf::NODE_PATH)?;
        let entry_prefix = keys::node_path_key_prefix(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            &group.workspace,
            &group.node_id,
        );
        // Oldest first, so an entry written for an older version is seen
        // (from `pending`: it is not committed yet) when the next one asks.
        group.legacy.sort_by(|a, b| a.0.cmp(&b.0));
        let mut pending: Vec<(HLC, String)> = Vec::new();
        for (revision, path) in &group.legacy {
            match entry_state(db, cf_node_path, &entry_prefix, &pending, revision, path)? {
                EntryState::Answers => continue,
                EntryState::Conflict => {
                    tracing::warn!(
                        workspace = %group.workspace,
                        node_id = %group.node_id,
                        revision = %revision,
                        embedded_path = %path,
                        "node_path backfill: NODE_PATH holds a different entry at the legacy \
                         blob's own revision; leaving it (the read rule settles the tie)"
                    );
                    counts.conflicts += 1;
                    continue;
                }
                EntryState::Missing => {}
            }
            let (t, r, b) = (scope.tenant_id, scope.repo_id, scope.branch);
            let (ws, id) = (group.workspace.as_str(), group.node_id.as_str());
            stage.push(
                keys::node_key_versioned(t, r, b, ws, id, revision),
                keys::node_path_key_versioned(t, r, b, ws, id, revision),
                path.clone(),
            );
            pending.push((*revision, path.clone()));
        }
    }
    writer.mark(PASS_NODE_PATH, &group.last_key);
    if writer.batch_full(stage.bytes()) {
        return stage.commit(db, scope, writer, counts);
    }
    Ok(true)
}

/// What `NODE_PATH` says about one legacy version.
#[derive(Debug, PartialEq, Eq)]
enum EntryState {
    /// Newest ≤ the revision (committed, or written earlier in this group) is
    /// a live entry naming the blob's path: nothing to write.
    Answers,
    /// A committed entry at EXACTLY the blob's revision names something else
    /// (another path, or a tombstone): never overwritten.
    Conflict,
    /// Anything else: the entry the version needs.
    Missing,
}

fn entry_state(
    db: &DB,
    cf_node_path: &impl rocksdb::AsColumnFamilyRef,
    entry_prefix: &[u8],
    pending: &[(HLC, String)],
    revision: &HLC,
    path: &str,
) -> Result<EntryState> {
    let committed = newest_at_or_before(db, cf_node_path, entry_prefix, Some(revision))?;
    let names_path = |v: &[u8]| !is_tombstone(v) && v == path.as_bytes();
    if let Some((c_rev, value)) = committed.as_ref() {
        if c_rev == revision {
            return Ok(if names_path(value) {
                EntryState::Answers
            } else {
                EntryState::Conflict
            });
        }
    }
    // `pending` holds only revisions below this one (one legacy version per
    // revision, oldest first), so the newer of the two answers.
    let answer: Option<&[u8]> = match (committed.as_ref(), pending.last()) {
        (Some((c_rev, value)), Some((p_rev, p_path))) => Some(if c_rev > p_rev {
            value.as_slice()
        } else {
            p_path.as_bytes()
        }),
        (Some((_, value)), None) => Some(value.as_slice()),
        (None, Some((_, p_path))) => Some(p_path.as_bytes()),
        (None, None) => None,
    };
    Ok(if answer.is_some_and(names_path) {
        EntryState::Answers
    } else {
        EntryState::Missing
    })
}
