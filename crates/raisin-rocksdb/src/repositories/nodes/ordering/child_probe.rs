//! `has_children` as an existence probe over `ORDERED_CHILDREN`.
//!
//! Every storage read populates `has_children`, so this runs once per node
//! read — per row of a SQL index scan, per child of a listing. It used to
//! reload the node (NODES seek, BRANCHES read, NODE_PATH seek, decode) only to
//! learn whether it was the root, and then collect EVERY child id of the parent
//! (one index entry per revision of every child) to test the list for
//! emptiness. Both are gone: the caller passes the path it already holds, and
//! the scan stops at the first child that is actually there.
//!
//! # Live confirmation
//!
//! For correct data a live `ORDERED_CHILDREN` entry means a live child, and the
//! answer is the same as the old full collection. The probe additionally
//! confirms the hit against `NODES` (one seek, no decode, no copy): a child
//! whose entry outlived it — the mis-keyed delete tombstone tracked in the
//! read/index plan — is skipped and the scan continues. That walk is capped at
//! [`DEAD_HIT_CAP`] dead entries; past it the probe answers `true`, which is
//! exactly the old answer (the dead entries are still live index entries), and
//! warns once per process.
//!
//! # Placement confirmation
//!
//! A live child is not necessarily still THIS parent's child: one moved away
//! behind a stale entry is live, just elsewhere. So a live hit is also checked
//! for being under the probed parent as of the read (see `child_placement.rs`)
//! — one `PATH_INDEX` seek in the common case — and a misplaced child counts
//! as a dead hit toward the same cap.

use super::super::helpers::is_tombstone;
use super::super::NodeRepositoryImpl;
use super::OrderedScanStart;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use std::sync::atomic::{AtomicBool, Ordering};

/// Dead index entries the probe will step over before it gives up and answers
/// as the unconfirmed index would.
const DEAD_HIT_CAP: usize = 64;

static DEAD_HIT_CAP_WARNED: AtomicBool = AtomicBool::new(false);

impl NodeRepositoryImpl {
    /// Whether `node_id` has at least one child at `max_revision` (HEAD when
    /// `None`).
    ///
    /// `node_path` is the node's path when the caller already has the node; it
    /// only decides whether this is the root, whose children are indexed under
    /// `"/"` rather than its id. Pass `None` when only the id is known — one
    /// `PATH_INDEX` seek answers that instead of a node reload.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) fn probe_has_children(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        node_path: Option<&str>,
        max_revision: Option<&HLC>,
    ) -> Result<bool> {
        let is_root = match node_path {
            Some(path) => path == "/",
            None => self.is_root_node_id(tenant_id, repo_id, branch, workspace, node_id)?,
        };
        let parent_key = if is_root { "/" } else { node_id };

        // The parent's own path, for placement confirmation: known up front
        // when the caller passed it or the parent is the root, else resolved
        // on the first live hit (`None` inside = unknown, skip confirmation).
        let mut parent_path: Option<Option<String>> = match node_path {
            Some(path) => Some(Some(path.to_string())),
            None if parent_key == "/" => Some(Some("/".to_string())),
            None => None,
        };

        let mut found = false;
        let mut dead_hits = 0usize;
        self.scan_ordered_children(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_key,
            OrderedScanStart::Beginning,
            false,
            max_revision,
            |child_id, _order_label, name| {
                if self.child_is_live(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    child_id,
                    max_revision,
                )? {
                    if parent_path.is_none() {
                        parent_path = Some(self.node_path_at(
                            tenant_id,
                            repo_id,
                            branch,
                            workspace,
                            node_id,
                            max_revision,
                        )?);
                    }
                    let placed = match parent_path.as_ref().and_then(Option::as_deref) {
                        // Unknown parent path: keep the index's answer.
                        None => true,
                        Some(path) => self.child_is_under(
                            tenant_id,
                            repo_id,
                            branch,
                            workspace,
                            child_id,
                            name,
                            path,
                            max_revision,
                        )?,
                    };
                    if placed {
                        found = true;
                        return Ok(false);
                    }
                }
                dead_hits += 1;
                if dead_hits >= DEAD_HIT_CAP {
                    if !DEAD_HIT_CAP_WARNED.swap(true, Ordering::Relaxed) {
                        tracing::warn!(
                            tenant_id,
                            repo_id,
                            branch,
                            workspace,
                            parent = parent_key,
                            dead_hits,
                            "has_children probe met {} ORDERED_CHILDREN entries whose child \
                             is gone or elsewhere; answering from the index unconfirmed. \
                             The parent's ordered-children index needs repair (further \
                             occurrences are not logged)",
                            DEAD_HIT_CAP
                        );
                    }
                    found = true;
                    return Ok(false);
                }
                Ok(true)
            },
        )?;
        Ok(found)
    }

    /// Whether the newest `NODES` version of `child_id` at `max_revision` is a
    /// live node rather than a tombstone (or absent). One seek; the blob is
    /// looked at in place, never decoded or copied.
    fn child_is_live(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        child_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<bool> {
        let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, child_id);
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        Ok(crate::mvcc_read::newest_at_or_before_with(
            &self.db,
            cf_nodes,
            &prefix,
            max_revision,
            |_, value| !is_tombstone(value),
        )?
        .unwrap_or(false))
    }

    /// Whether `node_id` is the workspace root, answered from the HEAD
    /// `PATH_INDEX` entry for `"/"` — the same question the old probe answered
    /// by reloading the node at HEAD and comparing its path.
    fn is_root_node_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<bool> {
        let prefix = keys::path_index_key_prefix(tenant_id, repo_id, branch, workspace, "/");
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        Ok(crate::mvcc_read::newest_at_or_before_with(
            &self.db,
            cf_path,
            &prefix,
            None,
            |_, value| !is_tombstone(value) && value == node_id.as_bytes(),
        )?
        .unwrap_or(false))
    }
}
