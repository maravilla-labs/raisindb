//! Reconstruct PATH_INDEX from NODE_PATH.
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
//! * **It reads NODE_PATH, which is the source of truth**, not a derived index.
//!   Nothing in the rebuild path clears NODE_PATH, which is why the damage
//!   above was recoverable at all.
//! * **It writes at each entry's OWN revision**, so it needs no branch head and
//!   cannot be defeated by a missing or stale branch record.
//! * **It never deletes.** An index repair that begins by deleting is the shape
//!   that caused the incident. Being write-only also makes it safely
//!   re-runnable: a second pass writes the same keys with the same values.
//!
//! # What it restores, and what it does not
//!
//! Only the NEWEST non-tombstone NODE_PATH entry per node — the CURRENT tree.
//! Deliberately not the history, and the reason is correctness rather than
//! cost. `get_node_id_by_path_as_of` skips a tombstone and keeps looking at
//! OLDER entries, so replaying a node's whole path history would make a moved
//! node answerable at the path it used to occupy, and a deleted node
//! answerable at all. Restoring the current tree cannot do either.
//!
//! So a path lookup AT AN OLD REVISION may still miss after this runs. Those
//! entries were destroyed by the clear; nothing can reconstruct them without
//! reintroducing the resurrection above.

use crate::{cf, cf_handle, keys, RocksDBStorage};
use raisin_error::Result;
use rocksdb::WriteBatch;
use serde::{Deserialize, Serialize};

/// What one workspace's repair did. Every node scanned lands in exactly one of
/// `entries_written`, `skipped_deleted` or `skipped_unreadable`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PathIndexRepairStats {
    pub workspace: String,
    /// NODE_PATH entries walked, every revision of every node.
    pub entries_read: usize,
    /// Distinct node ids seen.
    pub nodes_seen: usize,
    /// Forward PATH_INDEX entries written (or, in a dry run, that would be).
    pub entries_written: usize,
    /// Nodes whose newest entry is a tombstone: deleted, correctly absent from
    /// the forward index.
    pub skipped_deleted: usize,
    /// Older revisions of a node already decided by a newer entry.
    pub skipped_superseded: usize,
    /// Entries whose key or value could not be read. NOT ZERO IS A PROBLEM:
    /// each one is a node that will stay unreachable by path.
    pub skipped_unreadable: usize,
    pub dry_run: bool,
}

/// Rebuild `workspace`'s PATH_INDEX from its NODE_PATH entries.
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

    let cf_node_path = cf_handle(storage.db(), cf::NODE_PATH)?;
    let cf_path = cf_handle(storage.db(), cf::PATH_INDEX)?;

    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("node_path")
        .build_prefix();

    let iter = crate::prefix_scan(storage.db(), cf_node_path, prefix.clone());

    // Entries for one node id are contiguous and the revision is encoded
    // DESCENDING, so the first entry seen for an id is its newest and decides
    // the id outright — the same "first one wins" the node scans use.
    let mut current_node: Option<String> = None;
    let mut batch = WriteBatch::default();
    let mut pending = 0usize;

    for item in iter {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        if !key.starts_with(&prefix) {
            break;
        }
        stats.entries_read += 1;

        let suffix = &key[prefix.len()..];
        let Some(id_bytes) = suffix.split(|&b| b == 0).next() else {
            stats.skipped_unreadable += 1;
            continue;
        };
        let Ok(node_id) = std::str::from_utf8(id_bytes) else {
            stats.skipped_unreadable += 1;
            continue;
        };

        if current_node.as_deref() == Some(node_id) {
            stats.skipped_superseded += 1;
            continue;
        }
        current_node = Some(node_id.to_string());
        stats.nodes_seen += 1;

        // Newest entry is a tombstone: the node is deleted and belongs in no
        // forward entry. Writing its older paths back is precisely how a
        // deleted node becomes answerable again.
        if value.is_empty() || crate::repositories::is_node_tombstone(&value) {
            stats.skipped_deleted += 1;
            continue;
        }

        let Ok(path) = std::str::from_utf8(&value) else {
            tracing::warn!(node_id = %node_id, "path repair: NODE_PATH value is not UTF-8");
            stats.skipped_unreadable += 1;
            continue;
        };

        // The entry's OWN revision, never a branch head — that dependency is
        // what broke the rebuild this repairs.
        let revision = match keys::extract_revision_from_key(&key) {
            Ok(rev) => rev,
            Err(e) => {
                tracing::warn!(
                    node_id = %node_id,
                    error = %e,
                    "path repair: could not read the revision from a NODE_PATH key"
                );
                stats.skipped_unreadable += 1;
                continue;
            }
        };

        stats.entries_written += 1;
        if dry_run {
            continue;
        }

        let forward =
            keys::path_index_key_versioned(tenant_id, repo_id, branch, workspace, path, &revision);
        batch.put_cf(cf_path, forward, node_id.as_bytes());
        pending += 1;

        if pending >= 1000 {
            storage
                .db()
                .write(std::mem::take(&mut batch))
                .map_err(|e| {
                    raisin_error::Error::storage(format!("path repair batch write failed: {}", e))
                })?;
            pending = 0;
        }
    }

    if pending > 0 {
        storage.db().write(batch).map_err(|e| {
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
