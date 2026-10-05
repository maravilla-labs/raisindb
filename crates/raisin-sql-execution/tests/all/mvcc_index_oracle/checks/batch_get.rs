//! Plan Phase 4: the batched snapshot read (`get_many_for_read`) answers
//! every item exactly as one `get` / `get_by_path` per item does, at every
//! revision of every history the oracle drives — through every write funnel,
//! fork, merge, GC and replication replay.
//!
//! The comparison is against the per-item reference (`get_many_by_loop`, the
//! `sql.batched_fetch = false` path), not against the model: the other
//! templates already hold `get` to the model, and a known `get` defect (see
//! `expected.rs`) must not surface a second time under this name.

use super::{name, Checker};
use crate::mvcc_index_oracle::env::{REPO, TENANT, WS};
use crate::mvcc_index_oracle::model::Snapshot;
use raisin_storage::{
    get_many_by_loop, BatchReadItem, BranchScope, NodeRepository, ReadOpts, Storage,
};

impl Checker<'_> {
    pub async fn batch_get(&mut self, s: &Snapshot) {
        let mut items: Vec<BatchReadItem> = s
            .tree
            .nodes
            .keys()
            .map(|id| BatchReadItem::id(WS, id))
            .collect();
        items.extend(
            s.tree
                .nodes
                .keys()
                .map(|id| BatchReadItem::path(WS, s.tree.path(id))),
        );
        if let Some(seen) = self.seen_paths.get(&s.branch) {
            items.extend(seen.iter().map(|p| BatchReadItem::path(WS, p)));
        }
        items.push(BatchReadItem::id(WS, "oracle-never-created"));

        let nodes = self.env.storage.nodes();
        let scope = BranchScope::new(TENANT, REPO, &s.branch);
        let opts = ReadOpts::default();
        let snapshot = nodes.open_read_snapshot();
        let batched = nodes
            .get_many_for_read(scope, &items, &s.head, snapshot.as_ref(), opts.clone())
            .await;
        let reference = get_many_by_loop(nodes, scope, &items, &s.head, &opts).await;
        match (batched, reference) {
            (Ok(batched), Ok(reference)) => {
                for ((item, got), want) in items.iter().zip(batched).zip(reference) {
                    if got != want {
                        let brief = |n: &Option<raisin_models::nodes::Node>| {
                            n.as_ref().map(|n| (n.id.clone(), n.path.clone()))
                        };
                        self.report(
                            name::BATCH_GET,
                            s,
                            format!(
                                "{:?}: batched {:?}, get {:?}",
                                item.locator,
                                brief(&got),
                                brief(&want)
                            ),
                        );
                    }
                }
            }
            (batched, reference) => self.report(
                name::BATCH_GET,
                s,
                format!(
                    "read failed: batched {:?}, get {:?}",
                    batched.err(),
                    reference.err()
                ),
            ),
        }
    }
}
