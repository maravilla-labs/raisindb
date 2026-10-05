//! The replication apply path's baseline reader: the node as this replica
//! last stored it.
//!
//! Every replicated upsert diffs the incoming node against the stored one to
//! tombstone what it supersedes (old path, old property and reference values,
//! old geometries). That read used to scan the WHOLE branch's NODES column
//! family for the first key whose id segment matched — O(branch) per applied
//! op — and decoded the blob as a full `Node`. Repository-written blobs are
//! `StorageNode`s with no `path`, so the baseline's path came back as `""` and
//! the old PATH_INDEX entry was never tombstoned.
//!
//! Now:
//! - the read is one seek on `{t}\0{r}\0{b}\0{ws}\0nodes\0{id}\0`;
//! - the blob goes through the shared `deserialize_node_with_path`, which
//!   materializes a `StorageNode`'s path from NODE_PATH at the version's own
//!   revision — the ONE baseline decoder (Phases 7, 8 and 12 build on it);
//! - [`OperationApplicator::load_node_before`] answers "the newest version
//!   strictly below revision R", the baseline an out-of-order apply needs.
//!
//! # Where the node lives
//!
//! A replicated node names its workspace: every emitter stamps it (the
//! transaction commit, the repository capture, tree and cross-branch copy),
//! so a miss in it is a genuine "not stored here yet" (a create) and costs
//! nothing more. The fallback that scanned the branch when an op from an
//! older peer named none is gone with those peers (plan "Phase 11d": no
//! cluster runs a pre-v2 binary). Only an id-only delete
//! (`DeleteNodeSnapshot`) still has to find the workspace by scan
//! ([`WorkspaceHint::Unknown`], counted by [`unknown_workspace_scans`]).

use super::OperationApplicator;
use crate::mvcc_read::predecessor;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use std::sync::atomic::{AtomicU64, Ordering};

/// Where a node's stored versions are to be looked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceHint<'a> {
    /// The op named the workspace: a miss means the node is not stored.
    Explicit(&'a str),
    /// Nothing names the workspace (a delete op carries only the id): only the
    /// branch scan can find it.
    Unknown,
}

impl<'a> WorkspaceHint<'a> {
    /// The hint a replicated node's own `workspace` field gives.
    pub fn of(node: &'a Node) -> Self {
        Self::Explicit(super::node_workspace(node))
    }
}

static UNKNOWN_WORKSPACE_SCANS: AtomicU64 = AtomicU64::new(0);

/// How many baseline reads had NO workspace at all (an id-only delete) and
/// scanned the branch, since process start: every branch scan this path still
/// performs.
pub fn unknown_workspace_scans() -> u64 {
    UNKNOWN_WORKSPACE_SCANS.load(Ordering::Relaxed)
}

impl OperationApplicator {
    /// The newest stored version of `node_id`, or `None` when it is absent or
    /// its newest version is a tombstone.
    pub fn load_latest_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: WorkspaceHint<'_>,
        node_id: &str,
    ) -> Result<Option<Node>> {
        self.load_node_at_or_before(tenant_id, repo_id, branch, workspace, node_id, None)
    }

    /// The newest stored version of `node_id` STRICTLY below `revision` — the
    /// baseline for an op that may arrive after a newer one was applied — or
    /// `None` when there is none, or it is a tombstone.
    pub fn load_node_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: WorkspaceHint<'_>,
        node_id: &str,
        revision: &HLC,
    ) -> Result<Option<Node>> {
        let Some(bound) = predecessor(revision) else {
            return Ok(None);
        };
        self.load_node_at_or_before(tenant_id, repo_id, branch, workspace, node_id, Some(&bound))
    }

    /// The version an apply at `revision` supersedes, with the revision it is
    /// stored at: the one stored AT `revision` (a `versionable=false` write
    /// overwrites its node's revision in place, so the version it replaces
    /// sits at the very same key), else the newest strictly below. A replayed
    /// duplicate finds itself, which diffs empty.
    pub fn load_node_replaced_by(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: WorkspaceHint<'_>,
        node_id: &str,
        revision: &HLC,
    ) -> Result<Option<(HLC, Node)>> {
        self.load_versioned_at_or_before(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            Some(revision),
        )
    }

    fn load_node_at_or_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: WorkspaceHint<'_>,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<Node>> {
        Ok(self
            .load_versioned_at_or_before(
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_id,
                max_revision,
            )?
            .map(|(_, node)| node))
    }

    fn load_versioned_at_or_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: WorkspaceHint<'_>,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<(HLC, Node)>> {
        let found_in = match workspace {
            WorkspaceHint::Explicit(ws) => {
                return self.load_scoped(tenant_id, repo_id, branch, ws, node_id, max_revision)
            }
            WorkspaceHint::Unknown => {
                UNKNOWN_WORKSPACE_SCANS.fetch_add(1, Ordering::Relaxed);
                self.find_node_workspace(tenant_id, repo_id, branch, node_id)?
            }
        };
        match found_in {
            Some(ws) => self.load_scoped(tenant_id, repo_id, branch, &ws, node_id, max_revision),
            None => Ok(None),
        }
    }

    /// One seek: the newest version at or before `max_revision` in
    /// `workspace`, with the revision it is stored at — through the one
    /// baseline reader, `mvcc_read::node_version_at_or_before`.
    fn load_scoped(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<(HLC, Node)>> {
        Ok(crate::mvcc_read::node_version_at_or_before(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            max_revision,
        )?
        .and_then(|(revision, node)| node.map(|node| (revision, node))))
    }

    /// The workspace holding a LIVE `node_id` on this branch, found by scanning
    /// the branch's NODES keys. The slow path: only for id-only deletes.
    ///
    /// A workspace whose newest version of the id is a tombstone is skipped and
    /// the scan goes on: an id deleted in one workspace and live in another
    /// must resolve to the live one, or an id-only delete tombstones nothing.
    /// Keys sort newest-first within one (workspace, id), so the first key seen
    /// for a workspace carries its newest version.
    fn find_node_workspace(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_id: &str,
    ) -> Result<Option<String>> {
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .build_prefix();
        let mut dead_in: Option<Vec<u8>> = None;
        for item in crate::prefix_scan(&self.db, cf_nodes, &prefix) {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            if !key.starts_with(&prefix) {
                break;
            }
            // {t}\0{r}\0{b}\0{ws}\0nodes\0{id}\0{~rev}: the first six segments
            // are plain text; only the revision trailer may contain \0.
            let mut parts = key[prefix.len()..].splitn(4, |&b| b == 0);
            let (Some(ws), Some(kind), Some(id)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if kind == b"nodes" && id == node_id.as_bytes() {
                if dead_in.as_deref() == Some(ws) {
                    continue; // an older version under a newest-is-tombstone
                }
                if super::is_tombstone(&value) {
                    dead_in = Some(ws.to_vec());
                    continue;
                }
                return Ok(Some(String::from_utf8_lossy(ws).into_owned()));
            }
        }
        Ok(None)
    }
}
