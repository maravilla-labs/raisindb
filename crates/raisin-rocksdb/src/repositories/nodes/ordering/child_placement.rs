//! Is a live child still under the parent whose `ORDERED_CHILDREN` names it?
//!
//! An `ORDERED_CHILDREN` entry can go stale without its child dying: a move
//! that failed to tombstone the old parent's entry leaves the child live,
//! just somewhere else. The `has_children` probe confirms placement here before
//! it counts a hit, and the ORDERED_CHILDREN repair uses the same answer to
//! find the stale entries it tombstones.

use super::super::helpers::is_tombstone;
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::DB;

impl NodeRepositoryImpl {
    /// [`child_is_under`] over this repository's database.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn child_is_under(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        child_id: &str,
        name: &[u8],
        parent_path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<bool> {
        child_is_under(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            child_id,
            name,
            parent_path,
            max_revision,
        )
    }

    /// [`node_path_at`] over this repository's database.
    pub(super) fn node_path_at(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<String>> {
        node_path_at(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            max_revision,
        )
    }
}

/// Whether `child_id` is a child of the node at `parent_path` as of
/// `max_revision` (HEAD when `None`).
///
/// First, one `PATH_INDEX` seek at the path the entry implies —
/// `{parent_path}/{name}`, the name being the entry's value: if that path
/// names `child_id`, the child is there. Only when it does not (the child
/// moved, or a rename left the entry's name behind) is the child's own path
/// read, and its parent compared. A child whose path cannot be learned at
/// all is given the benefit of the doubt: the index's answer stands.
#[allow(clippy::too_many_arguments)]
pub(crate) fn child_is_under(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    child_id: &str,
    name: &[u8],
    parent_path: &str,
    max_revision: Option<&HLC>,
) -> Result<bool> {
    let name = String::from_utf8_lossy(name);
    let implied = if parent_path == "/" {
        format!("/{name}")
    } else {
        format!("{parent_path}/{name}")
    };
    let prefix = keys::path_index_key_prefix(tenant_id, repo_id, branch, workspace, &implied);
    let cf_path = cf_handle(db, cf::PATH_INDEX)?;
    let at_implied_path =
        crate::mvcc_read::newest_at_or_before_with(db, cf_path, &prefix, max_revision, |_, v| {
            !is_tombstone(v) && v == child_id.as_bytes()
        })?
        .unwrap_or(false);
    if at_implied_path {
        return Ok(true);
    }

    let child_path = node_path_at(
        db,
        tenant_id,
        repo_id,
        branch,
        workspace,
        child_id,
        max_revision,
    )?;
    Ok(child_path.is_none_or(|path| parent_of(&path) == parent_path))
}

/// A node's path as of `max_revision`, without decoding properties when it
/// can be helped. `None` when deleted by then or the path is unknown.
///
/// The Phase 10 read rule, through its one blob-less implementation
/// ([`crate::mvcc_read::current_path`]): the newer, by revision, of
/// `NODE_PATH`'s newest entry and the path a legacy full-`Node` blob embeds,
/// a disagreeing tie decided by `PATH_INDEX`. ("`NODE_PATH` first" alone
/// returned the pre-rename path of a node renamed through the pre-Phase-10
/// transaction writer.)
pub(crate) fn node_path_at(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    max_revision: Option<&HLC>,
) -> Result<Option<String>> {
    let scope = crate::mvcc_read::NodeScope {
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
    };
    Ok(crate::mvcc_read::current_path(db, scope, max_revision)?.and_then(|c| c.path))
}

/// `/a/b` -> `/a`; `/a` -> `/`.
fn parent_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/",
        Some((parent, _)) => parent,
    }
}
