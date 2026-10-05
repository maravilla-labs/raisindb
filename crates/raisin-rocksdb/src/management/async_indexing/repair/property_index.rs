//! Phase 7: the PROPERTY_INDEX rebuild that unlocks skip-unchanged writes,
//! and the sampling verify job that replaces their accidental self-healing.
//!
//! **Why a rebuild gates the delta writer.** A skip-unchanged write never
//! re-puts an entry the predecessor already holds. Every update used to re-put
//! all of them, which silently repaired any entry a writer had once missed —
//! above all the replica writer, which never wrote IS_A / HAS_MIXIN
//! membership. With skips, such a hole is permanent. So the delta writer only
//! skips on a branch whose index this node has rebuilt to completion with the
//! one writer (`property_index` state record `done`, under THIS node's id —
//! a peer's record, arriving in a checkpoint, never counts), and does a full
//! put everywhere else (`NodeRepositoryImpl::skip_unchanged_permitted`).
//!
//! **The rebuild** streams the branch's NODES, takes each node's newest
//! version and puts every entry it indexes ([`entries_of`]) AT THAT VERSION'S
//! REVISION, never at HEAD: an entry above the version's revision is one a
//! later in-place write cannot mask (plan item 8). Data-detected: an entry
//! that is already live AS OF the version's revision (its exact key, or —
//! kept by a skip-unchanged write — the group's newest key below it) is not
//! rewritten, so a run over a maintained index writes only what was missing.
//! A tombstone at the exact revision IS overwritten: the writer's entries are
//! the version's entries (one derivation, `entries_of`, over a canonical
//! hash), so such a tombstone is a hole, not a decision.
//!
//! **The verify** (`property_index_verify`) samples one node in
//! [`RepairOptions::sample_every`] and checks that every entry of its newest
//! version is live as of that version's revision — by an MVCC read of the
//! entry's group (`property_delta::entry_state_as_of`), because an entry kept
//! by skips lives at the revision of the version that first wrote it, which
//! history GC may have deleted. It writes nothing; a miss resets the branch's
//! rebuild state (the writer falls back to full puts at once) and queues the
//! rebuild (`run_repair`). A group too crowded to walk within
//! [`PROBE_MAX_KEYS`] counts as `inconclusive`, never as a miss.

use super::super::node_key_parse::parse_node_key;
use super::cursor::BoundedWriter;
use crate::indexing::property_delta::{entries_of, entry_state_as_of, EntryState};
use crate::indexing::IndexCtx;
use crate::repositories::nodes::helpers::is_tombstone;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{ReadOptions, DB};
use serde::{Deserialize, Serialize};

pub(super) const PASS_REBUILD: &str = "property_index";

/// Keys of other nodes one entry's MVCC probe may pass before giving up.
pub const PROBE_MAX_KEYS: usize = 4096;
pub(super) const PASS_VERIFY: &str = "property_index_verify";

/// What the rebuild or the verify found on one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyIndexCounts {
    /// Live nodes (newest version not a tombstone) seen.
    pub nodes: u64,
    /// Nodes whose entries were checked (all for the rebuild, a sample for
    /// the verify).
    pub sampled: u64,
    /// Entries checked.
    pub entries: u64,
    /// Entries not live where they should be (written by the rebuild; only
    /// counted by the verify).
    pub missing: u64,
    /// Entries the verify could not decide within [`PROBE_MAX_KEYS`] (a
    /// value shared by very many nodes). Never counted as missing.
    #[serde(default)]
    pub inconclusive: u64,
}

pub(super) struct Scope<'a> {
    pub(super) tenant_id: &'a str,
    pub(super) repo_id: &'a str,
    pub(super) branch: &'a str,
}

/// Stream the branch's NODES. `verify_every = None` rebuilds; `Some(n)`
/// verifies one node in `n`. Returns `false` when the run must stop early.
pub(super) fn property_index_pass(
    db: &DB,
    scope: &Scope<'_>,
    writer: &mut BoundedWriter<'_>,
    counts: &mut PropertyIndexCounts,
    verify_every: Option<u64>,
) -> Result<bool> {
    let pass = if verify_every.is_some() {
        PASS_VERIFY
    } else {
        PASS_REBUILD
    };
    writer.begin_pass(pass);
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
        Some(after) => {
            // Past every version of the node the cursor names (its group
            // prefix's successor; a revision byte can itself be 0xFF).
            match crate::prefix_successor(&after) {
                Some(next) => iter.seek(&next),
                None => return Ok(true),
            }
        }
        None => iter.seek(&branch_prefix),
    }

    let mut current: Option<Vec<u8>> = None;
    let mut versions: Vec<HLC> = Vec::new();
    let mut newest: Option<(String, String, HLC, Vec<u8>)> = None;
    let mut seen: u64 = 0;
    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        let Some((workspace, node_id, revision)) = parse_node_key(&branch_prefix, key) else {
            iter.next();
            continue;
        };
        let group = &key[..key.len() - 16];
        if current.as_deref() != Some(group) {
            if let Some(done) = newest.take() {
                if !finish(
                    db,
                    scope,
                    writer,
                    counts,
                    done,
                    &versions,
                    verify_every,
                    seen,
                )? {
                    return Ok(false);
                }
                seen += 1;
            }
            current = Some(group.to_vec());
            versions.clear();
            // Keys run newest first: the first of a group is the newest.
            newest = Some((
                workspace.to_string(),
                node_id.to_string(),
                revision,
                value.to_vec(),
            ));
        }
        versions.push(revision);
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    if let Some(done) = newest {
        if !finish(
            db,
            scope,
            writer,
            counts,
            done,
            &versions,
            verify_every,
            seen,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn finish(
    db: &DB,
    scope: &Scope<'_>,
    writer: &mut BoundedWriter<'_>,
    counts: &mut PropertyIndexCounts,
    (workspace, node_id, revision, value): (String, String, HLC, Vec<u8>),
    versions: &[HLC],
    verify_every: Option<u64>,
    seen: u64,
) -> Result<bool> {
    let pass = if verify_every.is_some() {
        PASS_VERIFY
    } else {
        PASS_REBUILD
    };
    let group_key = keys::node_key_prefix(
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        &workspace,
        &node_id,
    );
    if is_tombstone(&value) {
        return writer.checkpoint(pass, &group_key);
    }
    counts.nodes += 1;
    if verify_every.is_some_and(|every| seen % every.max(1) != 0) {
        return writer.checkpoint(pass, &group_key);
    }
    counts.sampled += 1;
    let node = crate::mvcc_read::deserialize_node_with_path(
        db,
        &value,
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        &workspace,
        &node_id,
        &revision,
        &revision,
    )?;
    let ctx = IndexCtx::new(scope.tenant_id, scope.repo_id, scope.branch, &workspace);
    let cf_property = cf_handle(db, cf::PROPERTY_INDEX)?;
    for entry in entries_of(&node) {
        counts.entries += 1;
        let state = entry_state(db, cf_property, &ctx, &entry, &node_id, &revision, versions)?;
        if verify_every.is_some() {
            match state {
                EntryState::Live => {}
                EntryState::Inconclusive => counts.inconclusive += 1,
                EntryState::Tombstone | EntryState::Absent => {
                    counts.missing += 1;
                    tracing::warn!(
                        workspace = %workspace,
                        node_id = %node_id,
                        property = %entry.name,
                        "property index verify: entry of the newest version is not live"
                    );
                }
            }
            continue;
        }
        if state == EntryState::Live {
            continue;
        }
        let key = entry.key(&ctx, &revision, &node_id);
        writer.put(cf::PROPERTY_INDEX, &key, node_id.as_bytes())?;
    }
    writer.checkpoint(pass, &group_key)
}

/// The state of `entry` for `node_id` as of `newest` (its newest version's
/// revision): first a point read at each of the node's retained version
/// revisions (`versions`, newest first — the common case, one read), then,
/// when none holds a key, the MVCC walk of the entry's group: an entry kept by
/// skip-unchanged writes sits at the revision of the version that first wrote
/// it, which history GC may have deleted.
#[allow(clippy::too_many_arguments)]
fn entry_state(
    db: &DB,
    cf_property: &rocksdb::ColumnFamily,
    ctx: &IndexCtx<'_>,
    entry: &crate::indexing::property_delta::PropertyEntry,
    node_id: &str,
    newest: &HLC,
    versions: &[HLC],
) -> Result<EntryState> {
    for revision in versions {
        let key = entry.key(ctx, revision, node_id);
        if let Some(value) = db
            .get_cf(cf_property, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
        {
            return Ok(if is_tombstone(&value) {
                EntryState::Tombstone
            } else {
                EntryState::Live
            });
        }
    }
    entry_state_as_of(db, cf_property, ctx, entry, node_id, newest, PROBE_MAX_KEYS)
}
