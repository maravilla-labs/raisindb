//! Reconstruct PATH_INDEX from each node's current path, by the read rule.
//!
//! # Why this exists separately from `rebuild_indexes`
//!
//! `rebuild_path_indexes` reconstructs the forward index from the NODE BLOBS,
//! which do not carry a path — `StorageNode` omits it so a subtree move is O(1)
//! in blob writes — so it first has to materialize each node's path out of
//! NODE_PATH *at the branch head*. When that lookup fails the scan silently
//! yields fewer nodes, and the rebuild has already committed to clearing the
//! keyspace, so it deletes the index instead of rebuilding it.
//!
//! That is not hypothetical. Measured 2026-09-09 on a live tenant:
//! `get_current_revision` returns `HLC::new(0, 0)` when a branch record is not
//! found (`helpers.rs`), `materialize_path` skips every entry NEWER than the
//! revision it is given, so at revision 0 every node in the tenant was skipped
//! — 109,277 of them — PATH_INDEX and PROPERTY_INDEX were cleared, nearly
//! nothing was written back, and every path lookup answered `NODE_NOT_FOUND`
//! against data that was entirely intact. A second run made it worse: it
//! cleared what the first had managed to write.
//!
//! This function exists to recover from exactly that, and is shaped so it
//! cannot repeat it:
//!
//! * **It decides each node's path by THE read rule**
//!   (`crate::mvcc_read::current_path`): the newer of `NODE_PATH` and the path
//!   a legacy full-`Node` blob embeds. Taking `NODE_PATH` alone — what this
//!   did before Phase 10 — disagreed with every reader while legacy blobs
//!   exist: a node renamed through the pre-Phase-10 `put_node` got its OLD
//!   path written back (a phantom at `/p1`, nothing at `/p2`), a node the
//!   transaction path created had no entry and was skipped, and one deleted
//!   and recreated through it read as deleted. It walks `NODES`, so every
//!   node with a record is considered whatever `NODE_PATH` holds. Nothing in
//!   the rebuild path clears `NODES` or `NODE_PATH`, which is why the damage
//!   above was recoverable at all.
//! * **It writes at the winning record's OWN revision**, so it needs no branch
//!   head and cannot be defeated by a missing or stale branch record.
//! * **It never deletes.** An index repair that begins by deleting is the shape
//!   that caused the incident. Being write-only also makes it safely
//!   re-runnable: a second pass writes the same keys with the same values.
//!
//! # What it restores, and what it does not
//!
//! Only each live node's CURRENT path — the current tree.
//! Deliberately not the history, and the reason is correctness rather than
//! cost. `get_node_id_by_path_as_of` skips a tombstone and keeps looking at
//! OLDER entries, so replaying a node's whole path history would make a moved
//! node answerable at the path it used to occupy, and a deleted node
//! answerable at all. Restoring the current tree cannot do either.
//!
//! So a path lookup AT AN OLD REVISION may still miss after this runs. Those
//! entries were destroyed by the clear; nothing can reconstruct them without
//! reintroducing the resurrection above.
//!
//! One residue: a disagreeing same-revision tie between `NODE_PATH` and a
//! legacy blob is settled by `PATH_INDEX` itself, which the clear destroyed,
//! so such a node is restored at its `NODE_PATH` path (the pre-Phase-10
//! answer).

use super::node_key_parse::{parse_node_key, workspace_nodes_prefix};
use crate::mvcc_read::{current_path, NodeScope};
use crate::repositories::nodes::helpers::is_tombstone;
use crate::{cf, cf_handle, keys, RocksDBStorage};
use raisin_error::Result;
use rocksdb::{ReadOptions, WriteBatch};
use serde::{Deserialize, Serialize};

/// What one workspace's repair did. Every node scanned lands in exactly one of
/// `entries_written`, `skipped_deleted` or `skipped_unreadable`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PathIndexRepairStats {
    pub workspace: String,
    /// `NODES` versions walked, every revision of every node.
    pub entries_read: usize,
    /// Distinct node ids seen.
    pub nodes_seen: usize,
    /// Forward PATH_INDEX entries written (or, in a dry run, that would be).
    pub entries_written: usize,
    /// Nodes whose newest record is a tombstone (or whose current path is):
    /// deleted, correctly absent from the forward index.
    pub skipped_deleted: usize,
    /// Older revisions of a node already decided by its newest record.
    pub skipped_superseded: usize,
    /// Nodes whose key or path could not be read. NOT ZERO IS A PROBLEM: each
    /// one is a node that will stay unreachable by path.
    pub skipped_unreadable: usize,
    pub dry_run: bool,
}

/// Rebuild `workspace`'s PATH_INDEX from each node's current path.
///
/// With `dry_run`, reports what it would write and writes nothing.
pub async fn repair_path_index(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    dry_run: bool,
) -> Result<PathIndexRepairStats> {
    let mut stats = PathIndexRepairStats {
        workspace: workspace.to_string(),
        dry_run,
        ..Default::default()
    };
    let db = storage.db();
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let cf_path = cf_handle(db, cf::PATH_INDEX)?;

    let branch_prefix = keys::branch_prefix(tenant_id, repo_id, branch);
    let prefix = workspace_nodes_prefix(tenant_id, repo_id, branch, workspace);
    let mut opts = ReadOptions::default();
    opts.set_total_order_seek(true);
    opts.fill_cache(false);
    if let Some(upper) = crate::prefix_successor(&prefix) {
        opts.set_iterate_upper_bound(upper);
    }
    let mut iter = db.raw_iterator_cf_opt(cf_nodes, opts);
    iter.seek(&prefix);

    // A node's versions are contiguous and the revision is encoded
    // DESCENDING, so the first version seen for an id is its newest.
    let mut current_node: Option<String> = None;
    let mut batch = WriteBatch::default();
    let mut pending = 0usize;

    while iter.valid() {
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            break;
        };
        stats.entries_read += 1;
        let Some((ws, node_id, _)) = parse_node_key(&branch_prefix, key) else {
            stats.skipped_unreadable += 1;
            iter.next();
            continue;
        };
        if ws != workspace || current_node.as_deref() == Some(node_id) {
            stats.skipped_superseded += usize::from(ws == workspace);
            iter.next();
            continue;
        }
        current_node = Some(node_id.to_string());
        stats.nodes_seen += 1;

        // Newest record is a tombstone: deleted, and in no forward entry.
        // Writing an older path back is how a deleted node becomes
        // answerable again.
        if is_tombstone(value) {
            stats.skipped_deleted += 1;
            iter.next();
            continue;
        }

        let scope = NodeScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
        };
        match current_path(db, scope, None)? {
            Some(current) => match current.path {
                Some(path) => {
                    stats.entries_written += 1;
                    if !dry_run {
                        // The winning record's OWN revision, never a branch
                        // head — that dependency is what broke the rebuild
                        // this repairs.
                        let forward = keys::path_index_key_versioned(
                            tenant_id,
                            repo_id,
                            branch,
                            workspace,
                            &path,
                            &current.revision,
                        );
                        batch.put_cf(cf_path, forward, node_id.as_bytes());
                        pending += 1;
                    }
                }
                None => stats.skipped_deleted += 1,
            },
            None => {
                tracing::warn!(
                    node_id = %node_id,
                    "path repair: a live record with no path in NODE_PATH or the blob"
                );
                stats.skipped_unreadable += 1;
            }
        }

        if pending >= 1000 {
            db.write(std::mem::take(&mut batch)).map_err(|e| {
                raisin_error::Error::storage(format!("path repair batch write failed: {}", e))
            })?;
            pending = 0;
        }
        iter.next();
    }
    iter.status()
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    if pending > 0 {
        db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("path repair final batch write failed: {}", e))
        })?;
    }

    tracing::info!(
        tenant = %tenant_id,
        repo = %repo_id,
        branch = %branch,
        workspace = %workspace,
        dry_run,
        entries_read = stats.entries_read,
        nodes_seen = stats.nodes_seen,
        entries_written = stats.entries_written,
        skipped_deleted = stats.skipped_deleted,
        skipped_unreadable = stats.skipped_unreadable,
        "PATH_INDEX repair complete"
    );

    Ok(stats)
}

/// Repair every workspace of a branch. One workspace's failure does not stop
/// the others — a repair that gives up halfway leaves the tenant in a worse
/// state than one that reports what it could not do.
pub async fn repair_path_index_all_workspaces(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    dry_run: bool,
) -> Result<Vec<PathIndexRepairStats>> {
    use raisin_storage::{RepoScope, Storage, WorkspaceRepository};

    let workspaces = storage
        .workspaces()
        .list(RepoScope::new(tenant_id, repo_id))
        .await?;

    let mut out = Vec::with_capacity(workspaces.len());
    for ws in workspaces {
        match repair_path_index(storage, tenant_id, repo_id, branch, &ws.name, dry_run).await {
            Ok(stats) => out.push(stats),
            Err(e) => {
                tracing::warn!(
                    workspace = %ws.name,
                    error = %e,
                    "path repair failed for this workspace; continuing with the rest"
                );
                out.push(PathIndexRepairStats {
                    workspace: ws.name.clone(),
                    dry_run,
                    ..Default::default()
                });
            }
        }
    }
    Ok(out)
}
