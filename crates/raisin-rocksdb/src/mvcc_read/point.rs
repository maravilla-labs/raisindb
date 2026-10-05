//! The two point reads every node reader shares: the id a path names, and a
//! node as of a revision.
//!
//! `get` / `get_by_path` call these over the live database ([`super::DbRead`]),
//! the batched reader over its snapshot-pinned iterators
//! ([`super::SnapshotRead`]). ONE body each, so the single and the batched
//! answers cannot drift: a change to the tombstone test, the seek, or the
//! decode lands in both at once.

use super::{deserialize_node_with_path_in, NodeScope, VersionedRead};
use crate::repositories::nodes::helpers::is_tombstone;
use crate::repositories::nodes::PropertiesMode;
use crate::{cf, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

/// A path's `PATH_INDEX` entry: its revision, and the node id it names —
/// `None` when the entry is a tombstone (the path was deleted or moved away;
/// merge's legacy `\x00` marker counts, through the shared `is_tombstone`).
pub(crate) type PathEntry = (HLC, Option<String>);

/// The newest `PATH_INDEX` entry for `path` at or before `max_revision`
/// (newest at all when `None`). `Ok(None)`: the path never had one.
pub(crate) fn path_index_entry_in(
    src: &mut impl VersionedRead,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    path: &str,
    max_revision: Option<&HLC>,
) -> Result<Option<PathEntry>> {
    let prefix = keys::path_index_key_prefix(tenant_id, repo_id, branch, workspace, path);
    src.newest_at_or_before_with(cf::PATH_INDEX, &prefix, max_revision, |revision, value| {
        let id = (!is_tombstone(value)).then(|| String::from_utf8_lossy(value).into_owned());
        (revision, id)
    })
}

/// A node's newest version at or before `at`: its `NODES` revision, and the
/// decoded node — `None` for a tombstone (deleted then). `Ok(None)`: the node
/// had no version at or before `at`. An entry above `at` (a later write, or
/// one stranded above HEAD) is never seen.
///
/// The path comes from THE path rule as of `at` (not the blob's revision: a
/// later ancestor move writes `NODE_PATH` above it), read through `src`.
pub(crate) fn node_version_in(
    src: &mut impl VersionedRead,
    scope: NodeScope<'_>,
    at: &HLC,
    mode: PropertiesMode,
) -> Result<Option<(HLC, Option<Node>)>> {
    let prefix = keys::node_key_prefix(
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
        scope.node_id,
    );
    let Some((revision, bytes)) = src.newest_at_or_before(cf::NODES, &prefix, Some(at))? else {
        return Ok(None);
    };
    if is_tombstone(&bytes) {
        return Ok(Some((revision, None)));
    }
    let node = deserialize_node_with_path_in(src, &bytes, scope, at, &revision, mode)?;
    Ok(Some((revision, Some(node))))
}

/// The newest record a read of the node at `at` depends on — its `NODES`
/// blob or its `NODE_PATH` entry, whichever is newer — from the keys alone,
/// nothing decoded. `None`: neither has an entry at or before `at`.
///
/// Two sources that agree on this revision hold the same versions of the
/// node. An in-place `versionable=false` rewrite keeps it (same keys, new
/// values), which is exactly the difference a pinned view exists to hide; any
/// other write at or below `at` raises it.
pub(crate) fn node_record_revision_in(
    src: &mut impl VersionedRead,
    scope: NodeScope<'_>,
    at: &HLC,
) -> Result<Option<HLC>> {
    let NodeScope {
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
    } = scope;
    let blob = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let blob = src.newest_at_or_before_with(cf::NODES, &blob, Some(at), |rev, _| rev)?;
    let path = keys::node_path_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let path = src.newest_at_or_before_with(cf::NODE_PATH, &path, Some(at), |rev, _| rev)?;
    Ok(blob.max(path))
}
