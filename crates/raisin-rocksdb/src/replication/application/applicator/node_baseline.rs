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
//! # The workspace is not always known
//!
//! An op's node carries its workspace — except from older peers, whose nodes
//! may not, and the apply path then DEFAULTS it to `"default"`. A miss in a
//! defaulted workspace cannot be told apart from "this node lives elsewhere",
//! so it falls back to the branch scan, counted and warned
//! ([`scoped_miss_fallbacks`]). The fallback is removed after a release in which
//! that counter stayed at zero cluster-wide. A miss in an EXPLICIT workspace is
//! a genuine "not stored here yet" (a create) and costs nothing more.

use super::OperationApplicator;
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
    /// The op named none and `"default"`-style defaulting chose this one: a
    /// miss falls back to the branch scan (see [`scoped_miss_fallbacks`]).
    Defaulted(&'a str),
    /// Nothing names the workspace (a delete op carries only the id): only the
    /// branch scan can find it.
    Unknown,
}

impl<'a> WorkspaceHint<'a> {
    /// The hint a replicated node's own `workspace` field gives.
    pub fn of(node: &'a Node) -> Self {
        match node.workspace.as_deref() {
            Some(ws) => Self::Explicit(ws),
            None => Self::Defaulted(super::node_workspace(node)),
        }
    }
}

static SCOPED_MISS_FALLBACKS: AtomicU64 = AtomicU64::new(0);

static UNKNOWN_WORKSPACE_SCANS: AtomicU64 = AtomicU64::new(0);

/// How many baseline reads missed in a DEFAULTED workspace and had to scan the
/// branch, since process start. Zero cluster-wide for a release is the signal
/// to delete the fallback.
pub fn scoped_miss_fallbacks() -> u64 {
    SCOPED_MISS_FALLBACKS.load(Ordering::Relaxed)
}

/// How many baseline reads had NO workspace at all (an id-only delete) and
/// scanned the branch, since process start. Reported beside
/// [`scoped_miss_fallbacks`]: together they are every branch scan this path
/// still performs.
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

    fn load_node_at_or_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: WorkspaceHint<'_>,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<Node>> {
        let found_in = match workspace {
            WorkspaceHint::Explicit(ws) => {
                return self.load_scoped(tenant_id, repo_id, branch, ws, node_id, max_revision)
            }
            WorkspaceHint::Defaulted(ws) => {
                if let Some(node) =
                    self.load_scoped(tenant_id, repo_id, branch, ws, node_id, max_revision)?
                {
                    return Ok(Some(node));
                }
                let found_in = self.find_node_workspace(tenant_id, repo_id, branch, node_id)?;
                SCOPED_MISS_FALLBACKS.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    node_id = %node_id,
                    defaulted_workspace = %ws,
                    found_in = ?found_in,
                    "replication baseline: node not found in its defaulted workspace; \
                     scanned the branch (op from a peer that did not name the workspace)"
                );
                found_in
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

    /// One seek: the newest version at or before `max_revision` in `workspace`.
    fn load_scoped(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<Node>> {
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
        let Some((revision, value)) =
            crate::mvcc_read::newest_at_or_before(&self.db, cf_nodes, &prefix, max_revision)?
        else {
            return Ok(None);
        };
        if super::is_tombstone(&value) {
            return Ok(None);
        }
        let mut node = crate::mvcc_read::deserialize_node_with_path(
            &self.db, &value, tenant_id, repo_id, branch, workspace, node_id, &revision,
        )?;
        // A repository-written blob carries no workspace, and callers (the
        // id-only delete) read it back from here: the key is the authority.
        node.workspace = Some(workspace.to_string());
        Ok(Some(node))
    }

    /// The workspace holding a LIVE `node_id` on this branch, found by scanning
    /// the branch's NODES keys. The slow path: only for ops that do not name
    /// the workspace.
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

/// The greatest HLC strictly below `revision`, or `None` below the first.
fn predecessor(revision: &HLC) -> Option<HLC> {
    match (revision.timestamp_ms, revision.counter) {
        (0, 0) => None,
        (ts, 0) => Some(HLC::new(ts - 1, u64::MAX)),
        (ts, counter) => Some(HLC::new(ts, counter - 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::predecessor;
    use raisin_hlc::HLC;

    #[test]
    fn predecessor_is_strictly_below() {
        assert_eq!(predecessor(&HLC::new(5, 3)), Some(HLC::new(5, 2)));
        assert_eq!(predecessor(&HLC::new(5, 0)), Some(HLC::new(4, u64::MAX)));
        assert_eq!(predecessor(&HLC::new(0, 0)), None);
        assert!(predecessor(&HLC::new(5, 0)).unwrap() < HLC::new(5, 0));
    }
}
