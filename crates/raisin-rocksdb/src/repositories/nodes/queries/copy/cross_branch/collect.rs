//! Source node-set collection (BFS, parents first) for cross-branch copy.

use super::super::super::super::NodeRepositoryImpl;
use super::{parent_path_of, CopyEntry, RootContext};
use raisin_error::Result;
use raisin_models::nodes::Node;
use std::collections::{HashMap, HashSet};

impl NodeRepositoryImpl {
    /// STEP 2 of `copy_nodes_across_branches_impl`: collect the deduplicated
    /// source node set. Returns the staged entries and the set of source ids
    /// (used by delete_missing pruning).
    pub(super) fn collect_cross_branch_entries(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        workspace: &str,
        root_ctxs: &[RootContext],
        recursive: bool,
        source_revision: Option<&raisin_hlc::HLC>,
    ) -> Result<(Vec<CopyEntry>, HashSet<String>)> {
        // ========== STEP 2: collect the source node set (BFS, parents first) ==========
        let mut entries: Vec<CopyEntry> = Vec::new();
        let mut src_ids: HashSet<String> = HashSet::new();
        let mut path_to_id: HashMap<String, String> = HashMap::new();

        for rc in root_ctxs {
            let set: Vec<(Node, usize)> = if recursive {
                self.scan_descendants_ordered_impl(
                    tenant_id,
                    repo_id,
                    source_branch,
                    workspace,
                    &rc.node.id,
                    source_revision,
                )?
            } else {
                vec![(rc.node.clone(), 0)]
            };

            for (node, depth) in set {
                path_to_id.insert(node.path.clone(), node.id.clone());
                // Overlapping roots may visit a node twice — copy it once.
                if !src_ids.insert(node.id.clone()) {
                    continue;
                }

                let (src_parent_id, dst_parent_id) = if depth == 0 {
                    (rc.src_parent_id.clone(), rc.dst_parent_id.clone())
                } else {
                    // Parents-first BFS guarantees the parent path was
                    // already collected; ids are preserved, so the parent id
                    // is the same on both branches.
                    let parent_path = parent_path_of(&node.path);
                    let pid = path_to_id
                        .get(parent_path.as_str())
                        .cloned()
                        .ok_or_else(|| {
                            raisin_error::Error::internal(format!(
                                "Parent '{}' not collected before child '{}'",
                                parent_path, node.path
                            ))
                        })?;
                    (pid.clone(), pid)
                };

                entries.push(CopyEntry {
                    node,
                    src_parent_id,
                    dst_parent_id,
                });
            }
        }
        Ok((entries, src_ids))
    }
}
