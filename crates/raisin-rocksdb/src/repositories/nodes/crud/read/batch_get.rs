//! `NodeRepository::get_many_for_read`: many nodes, one revision, one view.
//!
//! Inside ONE `spawn_blocking`, against ONE RocksDB snapshot (the statement's,
//! or one taken for the call):
//!
//! 1. the items are sorted, so every iterator moves forward through its CF;
//! 2. path items resolve to ids through PATH_INDEX, newest at or below the
//!    revision — a tombstone (`T` or merge's legacy `\x00`, the shared
//!    `is_tombstone`) means the path names nothing then;
//! 3. each distinct `(workspace, id)` reads its NODES blob, newest at or below
//!    the revision — a tombstone means deleted, and an entry ABOVE the
//!    revision (a later write, a stranded one) is never seen;
//! 4. the blob decodes through THE decoder with THE path rule (Phase 10:
//!    the newer of NODE_PATH and a legacy blob's embedded path), reading
//!    NODE_PATH and the tie-breaking PATH_INDEX through the same snapshot.
//!
//! One raw iterator per CF serves the whole batch ([`SnapshotRead`]), which is
//! what this buys over a loop of `get`: iterator and superversion setup once
//! per CF instead of once per key, and no async worker blocked on a cold read.
//! The answer per item is exactly `get` / `get_by_path` at the same revision
//! (`batch_get_equals_get_for_random_ids_and_revisions`). The steps are THE
//! single-read bodies (`mvcc_read::{path_index_entry_in, node_version_in}`),
//! run over the batch's iterators instead of a fresh one per key.
//!
//! # The statement's view decides CONTENT, the live database decides RECENCY
//!
//! A statement's other readers (table, prefix and tree pages, the index reads
//! that name the candidates) read the live database, later than the view was
//! pinned. A commit at a revision at or below the statement's can land in
//! between — an out-of-order local commit (HEAD's monotonic guard skips the
//! HEAD update, the data still lands) or a replicated peer's ops below the
//! local HEAD. Read only from the view, a row those readers return could name
//! a target the view lacks (a broken link next to the row naming it), or an
//! index candidate could decode to an older version that no longer matches.
//!
//! So when the database has moved since the pin, each item is also probed
//! live, keys only: if the live database holds a NEWER record at or below the
//! revision (a `PATH_INDEX` entry, a `NODES` blob or a `NODE_PATH` entry), the
//! item is read live. A `versionable=false` rewrite in place keeps its keys
//! and revision, so the view still answers for it — the guarantee the view
//! exists for. Regression tests:
//! `batch_get_reads_out_of_order_commit_landed_after_pin`,
//! `resolve_inlines_target_committed_out_of_order_after_pin`.

use super::super::super::storage_node::PropertiesMode;
use super::super::super::NodeRepositoryImpl;
use super::read_snapshot::RocksReadSnapshot;
use crate::mvcc_read::{
    node_record_revision_in, node_version_in, path_index_entry_in, NodeScope, PathEntry, Recorded,
    SnapshotRead, VersionedRead,
};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::{
    BatchReadItem, BranchScope, NodeLocator, PropertiesRead, ReadOpts, ReadSnapshot,
};
use rocksdb::{SnapshotWithThreadMode, DB};
use std::collections::BTreeMap;

/// Owned copy of a branch scope, to move into the blocking task.
struct Branch {
    tenant_id: String,
    repo_id: String,
    branch: String,
}

impl NodeRepositoryImpl {
    /// See the module docs.
    pub(crate) async fn get_many_for_read_impl(
        &self,
        scope: BranchScope<'_>,
        items: &[BatchReadItem],
        at: &HLC,
        snapshot: Option<&ReadSnapshot>,
        opts: ReadOpts,
    ) -> Result<Vec<Option<Node>>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let view = snapshot
            .and_then(ReadSnapshot::downcast_arc::<RocksReadSnapshot>)
            .filter(|view| view.is_of(&self.db));
        if snapshot.is_some() && view.is_none() {
            tracing::warn!("get_many_for_read: snapshot from another backend ignored");
        }
        let repo = self.clone();
        let branch = Branch {
            tenant_id: scope.tenant_id.to_string(),
            repo_id: scope.repo_id.to_string(),
            branch: scope.branch.to_string(),
        };
        let items = items.to_vec();
        let at = *at;

        tokio::task::spawn_blocking(move || match &view {
            Some(view) => {
                let check_live = view.moved_since_pin();
                let (nodes, seeks) =
                    repo.read_many(view.snapshot(), check_live, &branch, &items, &at, &opts)?;
                view.add_seeks(seeks);
                Ok(nodes)
            }
            None => {
                let snapshot = repo.db.snapshot();
                repo.read_many(&snapshot, false, &branch, &items, &at, &opts)
                    .map(|(nodes, _)| nodes)
            }
        })
        .await
        .map_err(|e| raisin_error::Error::storage(format!("batched read task failed: {e}")))?
    }

    /// The blocking body: every read through `snapshot`, checked against the
    /// live database when `check_live` (see the module docs). Returns the
    /// nodes and the iterator seeks it issued.
    fn read_many(
        &self,
        snapshot: &SnapshotWithThreadMode<'_, DB>,
        check_live: bool,
        branch: &Branch,
        items: &[BatchReadItem],
        at: &HLC,
        opts: &ReadOpts,
    ) -> Result<(Vec<Option<Node>>, u64)> {
        let mut src = SnapshotRead::new(&self.db, snapshot);
        let mut live = check_live.then(|| SnapshotRead::live(&self.db));
        let mode = match opts.properties {
            PropertiesRead::Load => PropertiesMode::Load,
            PropertiesRead::Skip => PropertiesMode::Skip,
        };

        // 1–2. Sorted, so PATH_INDEX (and later NODES/NODE_PATH) seeks move
        // forward; paths resolve to ids at the revision.
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by(|&a, &b| items[a].cmp(&items[b]));
        let mut ids: Vec<Option<String>> = vec![None; items.len()];
        for &i in &order {
            ids[i] = match &items[i].locator {
                NodeLocator::Id(id) => Some(id.clone()),
                NodeLocator::Path(path) => {
                    let ws = items[i].workspace.as_str();
                    let mut entry = branch.path_entry(&mut src, ws, path, at)?;
                    if let Some(live) = live.as_mut() {
                        let newer = branch.path_entry(live, ws, path, at)?;
                        if revision_of(&newer) > revision_of(&entry) {
                            entry = newer;
                        }
                    }
                    entry.and_then(|(_, id)| id)
                }
            };
        }

        // 3–4. Each distinct node once, in key order.
        let mut nodes: BTreeMap<(&str, &str), Option<Node>> = BTreeMap::new();
        for (item, id) in items.iter().zip(&ids) {
            if let Some(id) = id {
                nodes.insert((item.workspace.as_str(), id.as_str()), None);
            }
        }
        for ((workspace, id), slot) in nodes.iter_mut() {
            let scope = branch.node(workspace, id);
            *slot = match live.as_mut() {
                None => node_version_in(&mut src, scope, at, mode)?,
                Some(live) => {
                    // Keys only; the view's revisions are recorded by its own
                    // read, so the common case costs two extra seeks.
                    let live_revision = node_record_revision_in(live, scope, at)?;
                    let mut viewed = Recorded::new(&mut src);
                    let answer = node_version_in(&mut viewed, scope, at, mode);
                    if live_revision > viewed.newest() {
                        node_version_in(live, scope, at, mode)?
                    } else {
                        answer?
                    }
                }
            }
            .and_then(|(_, node)| node);
            if opts.has_children {
                if let Some(node) = slot.as_mut() {
                    node.has_children = Some(self.probe_has_children(
                        &branch.tenant_id,
                        &branch.repo_id,
                        &branch.branch,
                        workspace,
                        &node.id,
                        Some(&node.path),
                        Some(at),
                    )?);
                }
            }
        }
        let seeks = src.seeks() + live.as_ref().map_or(0, SnapshotRead::seeks);
        tracing::trace!(
            items = items.len(),
            distinct = nodes.len(),
            seeks,
            "get_many_for_read"
        );

        let out = items
            .iter()
            .zip(&ids)
            .map(|(item, id)| {
                let id = id.as_deref()?;
                nodes
                    .get(&(item.workspace.as_str(), id))
                    .and_then(Option::clone)
            })
            .collect();
        Ok((out, seeks))
    }
}

/// The revision of a `PATH_INDEX` answer (`None`: no entry).
fn revision_of(entry: &Option<PathEntry>) -> Option<HLC> {
    entry.as_ref().map(|(revision, _)| *revision)
}

impl Branch {
    fn node<'a>(&'a self, workspace: &'a str, node_id: &'a str) -> NodeScope<'a> {
        NodeScope {
            tenant_id: &self.tenant_id,
            repo_id: &self.repo_id,
            branch: &self.branch,
            workspace,
            node_id,
        }
    }

    /// `get_by_path`'s PATH_INDEX step, through `src`.
    fn path_entry(
        &self,
        src: &mut impl VersionedRead,
        workspace: &str,
        path: &str,
        at: &HLC,
    ) -> Result<Option<PathEntry>> {
        path_index_entry_in(
            src,
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
            workspace,
            path,
            Some(at),
        )
    }
}
