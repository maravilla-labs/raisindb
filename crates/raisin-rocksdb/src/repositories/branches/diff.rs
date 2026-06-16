//! Per-node branch diff (added / modified / deleted relative to the merge-base).
//!
//! Unlike [`calculate_divergence`](super::BranchRepositoryImpl::calculate_divergence),
//! which returns only ahead/behind counts, this enumerates exactly which nodes
//! changed since two branches diverged. It walks the branch's revision DAG back
//! to the common ancestor and aggregates `RevisionMeta.changed_nodes` — cost is
//! O(commits since fork), not O(total nodes) — then resolves each node's path.

use raisin_context::{BranchDiff, NodeDiffInfo};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::tree::ChangeOperation;
use raisin_storage::RevisionRepository;
use std::collections::{HashMap, HashSet, VecDeque};

use super::BranchRepositoryImpl;

impl BranchRepositoryImpl {
    /// Compute the per-node diff of `branch` relative to `base_branch`'s merge-base.
    ///
    /// Returns the nodes that changed on `branch` since it diverged from
    /// `base_branch`, classified into `added` / `modified` / `deleted`. This is
    /// the "publish status" view: e.g. `diff_branches("publish", "main")` lists
    /// the unpublished edits on `publish`.
    pub async fn diff_branches(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        base_branch: &str,
    ) -> Result<BranchDiff> {
        use crate::repositories::revisions::RevisionRepositoryImpl;
        use raisin_storage::BranchRepository;

        let current = self
            .get_branch(tenant_id, repo_id, branch)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", branch))
            })?;

        // Merge-base between the two branches.
        let divergence = self
            .calculate_divergence(tenant_id, repo_id, branch, base_branch)
            .await?;

        let rev_repo = RevisionRepositoryImpl::new(
            self.db.clone(),
            format!("branch-diff-{}-{}", tenant_id, repo_id),
        );

        // BFS from the branch HEAD back to the merge-base, following both parent
        // and merge_parent. Keep the newest change per (node, locale) — BFS visits
        // HEAD first, so `or_insert` keeps the most recent operation/revision.
        let mut changes: HashMap<(String, Option<String>), (String, ChangeOperation, HLC)> =
            HashMap::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(current.head);
        visited.insert(current.head);

        while let Some(revision) = queue.pop_front() {
            if revision == divergence.common_ancestor {
                continue;
            }
            let Some(meta) = rev_repo
                .get_revision_meta(tenant_id, repo_id, &revision)
                .await?
            else {
                continue;
            };

            for change in &meta.changed_nodes {
                let key = (change.node_id.clone(), change.translation_locale.clone());
                changes.entry(key).or_insert((
                    change.workspace.clone(),
                    change.operation,
                    revision,
                ));
            }

            if let Some(parent) = meta.parent {
                if visited.insert(parent) {
                    queue.push_back(parent);
                }
            }
            if let Some(merge_parent) = meta.merge_parent {
                if visited.insert(merge_parent) {
                    queue.push_back(merge_parent);
                }
            }
        }

        // Resolve paths and bucket by operation.
        let cf_nodes = crate::cf_handle(&self.db, crate::cf::NODES)?;
        let mut diff = BranchDiff {
            common_ancestor: divergence.common_ancestor,
            added: Vec::new(),
            modified: Vec::new(),
            deleted: Vec::new(),
        };

        for ((node_id, locale), (workspace, operation, revision)) in changes {
            // Resolve the node's path at the revision where it last changed
            // (mirrors RevisionRepository::list_changed_nodes). A tombstone or a
            // missing version yields None (e.g. some deletes).
            let node_key = crate::keys::node_key_versioned(
                tenant_id, repo_id, branch, &workspace, &node_id, &revision,
            );
            let path = match self
                .db
                .get_cf(cf_nodes, &node_key)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            {
                Some(bytes) if !bytes.starts_with(b"TOMBSTONE") => {
                    rmp_serde::from_slice::<raisin_models::nodes::Node>(&bytes)
                        .ok()
                        .map(|n| n.path)
                }
                _ => None,
            };

            let operation_str = match operation {
                ChangeOperation::Added => "added",
                ChangeOperation::Modified => "modified",
                ChangeOperation::Deleted => "deleted",
                ChangeOperation::Reordered => "reordered",
            }
            .to_string();

            let info = NodeDiffInfo {
                node_id,
                workspace,
                path,
                operation: operation_str,
                translation_locale: locale,
            };

            match operation {
                ChangeOperation::Added => diff.added.push(info),
                ChangeOperation::Deleted => diff.deleted.push(info),
                ChangeOperation::Modified | ChangeOperation::Reordered => diff.modified.push(info),
            }
        }

        Ok(diff)
    }
}
