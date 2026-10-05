//! A node's path as of a read: THE path read rule (plan Phase 10).
//!
//! Two sources can name a node's path:
//!
//! - `NODE_PATH` — `{t}\0{r}\0{b}\0{ws}\0{id}\0{~rev}` → path, written by
//!   the one record writer (`crud::indexing::node_record`) on every node write
//!   — repository, transaction, merge, replication apply — and by every move;
//! - the path EMBEDDED in a legacy full-`Node` blob — what the transaction
//!   write path (with NO `NODE_PATH` entry), merge and the replication
//!   applicators stored before Phases 10/10b. Nothing writes such blobs any
//!   more; existing databases keep them, so this rule stays forever.
//!
//! The rule: **the newer of the two, by revision.** `NODE_PATH`'s newest entry
//! at or below the read, against the embedded path at the blob's own revision.
//!
//! **A tie is decided by `PATH_INDEX`, not by either source.** Both sources at
//! ONE revision and naming DIFFERENT paths is two writers in one transaction
//! (or one in-place `versionable=false` write), and neither order is safe to
//! assume:
//!
//! - `put_node(c)` then `move_node_tree(ancestor)` in one transaction (what a
//!   pre-Phase-10 binary did, and what any older database still holds): the
//!   legacy blob at R embeds the PRE-move path, the move's `NODE_PATH` at R the
//!   moved one. "The blob wins" read the stale path.
//! - a legacy `put_node` rename of a `versionable=false` node, in place at its
//!   reused revision R, over a repository-written `NODE_PATH` at R: the blob
//!   holds the NEW path. "`NODE_PATH` wins" read the stale one.
//!
//! In both, the last writer at R also wrote `PATH_INDEX(its path, R) = id` and
//! tombstoned the other path at R, so the rule asks it: the embedded path wins
//! a tie exactly when `PATH_INDEX` maps it to this node AT that revision. Only
//! a disagreeing tie pays for the lookup; a tie that agrees costs nothing.
//!
//! Why not "`NODE_PATH` wins": a node created through the repository (entry
//! `p1` at `r1`) and then renamed through a pre-Phase-10 `put_node` (blob `p2`
//! at `r2`, no entry) read back `p1` — forever, and at `r2` too.
//!
//! Why not "the blob wins": a later ancestor move writes `NODE_PATH` for every
//! descendant ABOVE its blob's revision; the blob's path is the pre-move one.
//!
//! (It also made a downgrade to the release before the Phase 10 writer safe.
//! Phase 10b dropped that rollout by owner decision: the one format is
//! written everywhere at once, and a binary older than this rule cannot open
//! a database this one has written to.)

use super::source::{DbRead, VersionedRead};
use crate::keys;
use crate::repositories::nodes::helpers::is_tombstone;
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;
use std::cmp::Ordering;

/// A path a full-`Node` blob embeds, with the revision of that blob.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EmbeddedPath<'a> {
    pub(crate) blob_revision: &'a HLC,
    pub(crate) path: &'a str,
}

/// The node a path is being decided for.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NodeScope<'a> {
    pub(crate) tenant_id: &'a str,
    pub(crate) repo_id: &'a str,
    pub(crate) branch: &'a str,
    pub(crate) workspace: &'a str,
    pub(crate) node_id: &'a str,
}

/// Whether a blob at `blob_revision` CAN beat the `NODE_PATH` entry at
/// `indexed_revision` (`None`: no entry at or below the read) — i.e. it is
/// not strictly older. The cheap gate: a reader skips decoding the blob for
/// its path when this is false.
pub(crate) fn embedded_path_may_win(indexed_revision: Option<&HLC>, blob_revision: &HLC) -> bool {
    indexed_revision.is_none_or(|indexed| indexed <= blob_revision)
}

/// Whether the embedded path beats the `NODE_PATH` entry `indexed`
/// (`(revision, value)`, the newest at or below the read).
///
/// The rule in one place: every reader that has both — the decoder, the
/// current-path resolver ([`current_path`]), the PATH_INDEX repair — asks
/// here. See the module docs for the tie.
pub(crate) fn embedded_path_wins(
    db: &DB,
    scope: NodeScope<'_>,
    indexed: Option<(&HLC, &[u8])>,
    embedded: EmbeddedPath<'_>,
) -> Result<bool> {
    embedded_path_wins_in(&mut DbRead(db), scope, indexed, embedded)
}

/// [`embedded_path_wins`] over any read source — a batch's snapshot-pinned
/// iterators ([`super::SnapshotRead`]) or the live database.
pub(crate) fn embedded_path_wins_in(
    src: &mut impl VersionedRead,
    scope: NodeScope<'_>,
    indexed: Option<(&HLC, &[u8])>,
    embedded: EmbeddedPath<'_>,
) -> Result<bool> {
    let Some((indexed_revision, indexed_value)) = indexed else {
        return Ok(true);
    };
    match indexed_revision.cmp(embedded.blob_revision) {
        Ordering::Less => Ok(true),
        Ordering::Greater => Ok(false),
        Ordering::Equal if indexed_value == embedded.path.as_bytes() => Ok(true),
        Ordering::Equal => path_index_confirms(src, scope, embedded),
    }
}

/// `PATH_INDEX` maps `embedded.path` to this node at exactly the blob's
/// revision: the writer that wrote last at that revision wrote this path.
fn path_index_confirms(
    src: &mut impl VersionedRead,
    scope: NodeScope<'_>,
    embedded: EmbeddedPath<'_>,
) -> Result<bool> {
    let prefix = keys::path_index_key_prefix(
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
        embedded.path,
    );
    let entry =
        src.newest_at_or_before(crate::cf::PATH_INDEX, &prefix, Some(embedded.blob_revision))?;
    Ok(matches!(
        entry,
        Some((rev, id)) if rev == *embedded.blob_revision && id == scope.node_id.as_bytes()
    ))
}

/// A node's path as of `read_at`, by the rule above.
///
/// The ONE implementation for a reader that already holds the node's blob:
/// the repository read path, the transaction read path, replication's
/// baseline reader and the index rebuilds all resolve a node's path through
/// here (most of them via [`super::deserialize_node_with_path`]). `embedded`
/// is the blob's own path when the blob carries one.
///
/// # Errors
/// When `NODE_PATH` decides (no embedded path, or a newer entry):
/// - its newest entry at or before `read_at` is a tombstone (deleted);
/// - there is no entry at or before `read_at`;
/// - the stored path is not UTF-8.
#[allow(clippy::too_many_arguments)]
pub(crate) fn materialize_path(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    read_at: &HLC,
    embedded: Option<EmbeddedPath<'_>>,
) -> Result<String> {
    let scope = NodeScope {
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
    };
    materialize_path_in(&mut DbRead(db), scope, read_at, embedded)
}

/// [`materialize_path`] over any read source: the ONE body of the rule for a
/// reader holding a blob, whether it reads the live database or a batch's
/// snapshot.
pub(crate) fn materialize_path_in(
    src: &mut impl VersionedRead,
    scope: NodeScope<'_>,
    read_at: &HLC,
    embedded: Option<EmbeddedPath<'_>>,
) -> Result<String> {
    let NodeScope {
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
    } = scope;
    let prefix = keys::node_path_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let indexed = src.newest_at_or_before(crate::cf::NODE_PATH, &prefix, Some(read_at))?;

    if let Some(embedded) = embedded {
        let indexed_ref = indexed.as_ref().map(|(rev, v)| (rev, v.as_slice()));
        if embedded_path_wins_in(src, scope, indexed_ref, embedded)? {
            return Ok(embedded.path.to_string());
        }
    }

    match indexed {
        Some((_, value)) if is_tombstone(&value) => Err(raisin_error::Error::storage(format!(
            "Node {} was deleted (tombstone in NODE_PATH)",
            node_id
        ))),
        Some((_, value)) => String::from_utf8(value)
            .map_err(|e| raisin_error::Error::storage(format!("Invalid path encoding: {}", e))),
        None => Err(raisin_error::Error::storage(format!(
            "Path not found for node_id={} at revision={}",
            node_id, read_at
        ))),
    }
}

/// A node's current path and the revision of the record that decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CurrentPath {
    /// The revision of the winning source: the blob's, or the entry's.
    pub(crate) revision: HLC,
    /// `None` when the winning `NODE_PATH` entry is a tombstone (deleted).
    pub(crate) path: Option<String>,
}

/// The rule for a reader that holds no blob: a node's path as of
/// `max_revision` (`None`: newest), without decoding properties.
///
/// The blob is decoded only when it is not older than the entry (or there is
/// none) — otherwise the entry decides without touching it. A tombstoned
/// newest blob leaves the decision to `NODE_PATH`. `Ok(None)`: neither source
/// knows the node.
pub(crate) fn current_path(
    db: &DB,
    scope: NodeScope<'_>,
    max_revision: Option<&HLC>,
) -> Result<Option<CurrentPath>> {
    let NodeScope {
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
    } = scope;
    let prefix = keys::node_path_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let cf_node_path = crate::cf_handle(db, crate::cf::NODE_PATH)?;
    let indexed = super::newest_at_or_before(db, cf_node_path, &prefix, max_revision)?;
    let indexed_revision = indexed.as_ref().map(|(rev, _)| *rev);

    let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let cf_nodes = crate::cf_handle(db, crate::cf::NODES)?;
    let blob = super::newest_at_or_before_with(db, cf_nodes, &prefix, max_revision, |rev, v| {
        if is_tombstone(v) || !embedded_path_may_win(indexed_revision.as_ref(), &rev) {
            return None;
        }
        super::embedded_path_of(v).map(|path| (rev, path))
    })?
    .flatten();

    if let Some((blob_revision, path)) = blob {
        let embedded = EmbeddedPath {
            blob_revision: &blob_revision,
            path: &path,
        };
        let indexed_ref = indexed.as_ref().map(|(rev, v)| (rev, v.as_slice()));
        if embedded_path_wins(db, scope, indexed_ref, embedded)? {
            return Ok(Some(CurrentPath {
                revision: blob_revision,
                path: Some(path),
            }));
        }
    }

    Ok(indexed.map(|(revision, value)| CurrentPath {
        revision,
        path: (!is_tombstone(&value)).then(|| String::from_utf8_lossy(&value).into_owned()),
    }))
}
