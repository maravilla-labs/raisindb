//! Node listing and counting operations

use super::super::super::helpers::is_tombstone;
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use std::collections::{HashMap, HashSet, VecDeque};

impl NodeRepositoryImpl {
    /// List all nodes in a workspace
    pub(in crate::repositories::nodes) async fn list_all_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<Node>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .build_prefix();

        let cf = cf_handle(&self.db, cf::NODES)?;
        let prefix_clone = prefix.clone();
        let iter = crate::prefix_scan(&self.db, cf, prefix);

        let mut nodes_map: HashMap<String, Node> = HashMap::new();
        let mut deleted_nodes: HashSet<String> = HashSet::new();

        tracing::debug!(
            "list_all_impl: tenant={}, repo={}, branch={}, workspace={}, max_revision={:?}",
            tenant_id,
            repo_id,
            branch,
            workspace,
            max_revision
        );

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            let revision = match keys::extract_revision_from_key(&key) {
                Ok(rev) => rev,
                Err(e) => {
                    eprintln!("Skipping key with invalid revision: {}", e);
                    continue;
                }
            };

            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() < 6 {
                eprintln!(
                    "Skipping key with unexpected format (parts={}): {:?}",
                    parts.len(),
                    String::from_utf8_lossy(&key)
                );
                continue;
            }
            let node_id = String::from_utf8_lossy(parts[5]);

            // Skip revisions beyond max_revision
            if let Some(max_rev) = max_revision {
                if &revision > max_rev {
                    tracing::debug!(
                        "  SKIP: node_id={}, revision={} > max_revision={}",
                        node_id,
                        revision,
                        max_rev
                    );
                    continue;
                } else {
                    tracing::debug!(
                        "  KEEP: node_id={}, revision={} <= max_revision={}",
                        node_id,
                        revision,
                        max_rev
                    );
                }
            } else {
                tracing::debug!(
                    "  KEEP: node_id={}, revision={} (no max_revision)",
                    node_id,
                    revision
                );
            }

            // Handle tombstones
            if is_tombstone(&value) {
                let node_id_str = node_id.to_string();
                tracing::debug!(
                    "  TOMBSTONE: node_id={} at revision {} - marking as deleted",
                    node_id_str,
                    revision
                );
                deleted_nodes.insert(node_id_str);
                continue;
            }

            let node_id = node_id.to_string();

            if deleted_nodes.contains(&node_id) {
                tracing::debug!(
                    "  SKIP: node_id={} at revision {} (deleted in newer revision)",
                    node_id,
                    revision
                );
                continue;
            }

            // Only add if we haven't seen this node_id yet (first = newest revision within bounds)
            if let std::collections::hash_map::Entry::Vacant(entry) =
                nodes_map.entry(node_id.clone())
            {
                let node = self.deserialize_node_with_path(
                    &value, tenant_id, repo_id, branch, workspace, &node_id, &revision,
                )?;
                tracing::debug!(
                    "  ADDED: node_id={}, path={}, revision={}",
                    node_id,
                    node.path,
                    revision
                );
                entry.insert(node);
            } else {
                tracing::debug!(
                    "  DUPLICATE: node_id={} already in map (older revision)",
                    node_id
                );
            }
        }

        tracing::debug!(
            "list_all_impl: collected {} nodes, now sorting by ordered children index",
            nodes_map.len()
        );

        let ordered_nodes = self
            .order_nodes_by_tree_traversal(
                tenant_id,
                repo_id,
                branch,
                workspace,
                nodes_map,
                max_revision,
            )
            .await?;

        tracing::debug!(
            "list_all_impl: returning {} ordered nodes",
            ordered_nodes.len()
        );

        Ok(ordered_nodes)
    }

    /// Count all nodes in a workspace without deserializing node data
    ///
    /// Optimized version of list_all that only counts keys.
    /// Memory: O(1) - only stores count and deduplication set
    pub(in crate::repositories::nodes) async fn count_all_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        max_revision: Option<&HLC>,
    ) -> Result<usize> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .build_prefix();

        let cf = cf_handle(&self.db, cf::NODES)?;
        let prefix_clone = prefix.clone();
        let iter = crate::prefix_scan(&self.db, cf, prefix);

        let mut seen_nodes = HashSet::new();
        let mut count = 0usize;

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::Backend(e.to_string()))?;

            // `prefix_iterator_cf` positions at the prefix but does not STOP at
            // its end (cf::NODES has no prefix extractor), so without this guard
            // the count ran on into every later workspace, branch and repository
            // in the column family. An unfiltered COUNT(*) over a four-node
            // workspace answered 639.
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let revision = match keys::extract_revision_from_key(&key) {
                Ok(rev) => rev,
                Err(_) => continue,
            };

            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() < 6 {
                continue;
            }
            let node_id = parts[5];

            if let Some(max_rev) = max_revision {
                if &revision > max_rev {
                    continue;
                }
            }

            if is_tombstone(&value) {
                seen_nodes.insert(String::from_utf8_lossy(node_id).to_string());
                continue;
            }

            if seen_nodes.contains(String::from_utf8_lossy(node_id).as_ref()) {
                continue;
            }

            seen_nodes.insert(String::from_utf8_lossy(node_id).to_string());
            count += 1;
        }

        Ok(count)
    }

    /// Order nodes using depth-first traversal of the ORDERED_CHILDREN index
    ///
    /// Performs an in-memory depth-first traversal starting from root nodes,
    /// using the ORDERED_CHILDREN index to determine the order of siblings.
    async fn order_nodes_by_tree_traversal(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        nodes_map: HashMap<String, Node>,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<Node>> {
        // Build parent_id -> children map
        let path_to_id: HashMap<String, String> = nodes_map
            .values()
            .map(|n| (n.path.clone(), n.id.clone()))
            .collect();

        let mut parent_to_children: HashMap<String, Vec<Node>> = HashMap::new();
        let mut root_nodes = Vec::new();

        for node in nodes_map.into_values() {
            if node.parent.is_none() || node.parent.as_deref() == Some("/") {
                root_nodes.push(node);
            } else {
                let parent_path = node.path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
                let parent_path = if parent_path.is_empty() {
                    "/"
                } else {
                    parent_path
                };

                if let Some(parent_id) = path_to_id.get(parent_path) {
                    parent_to_children
                        .entry(parent_id.clone())
                        .or_default()
                        .push(node);
                } else {
                    root_nodes.push(node);
                }
            }
        }

        // Sort root nodes by their order in ORDERED_CHILDREN index
        let root_ids = self
            .get_ordered_child_ids(tenant_id, repo_id, branch, workspace, "/", max_revision)
            .await?;

        let mut ordered_root_nodes = Vec::new();
        for root_id in root_ids {
            if let Some(pos) = root_nodes.iter().position(|n| n.id == root_id) {
                ordered_root_nodes.push(root_nodes.remove(pos));
            }
        }
        ordered_root_nodes.extend(root_nodes);

        // Depth-first traversal
        let mut result = Vec::new();
        let mut stack: VecDeque<Node> = VecDeque::from(ordered_root_nodes);

        while let Some(node) = stack.pop_front() {
            if node.path != "/" {
                result.push(node.clone());
            }

            let child_ids = self
                .get_ordered_child_ids(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node.id,
                    max_revision,
                )
                .await?;

            for child_id in child_ids.iter().rev() {
                if let Some(children) = parent_to_children.get(&node.id) {
                    if let Some(child_node) = children.iter().find(|c| c.id == *child_id) {
                        stack.push_front(child_node.clone());
                    }
                }
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod count_all_tests {
    use raisin_models::nodes::Node;
    use raisin_storage::{
        BranchRepository, CreateNodeOptions, NodeRepository, Storage, StorageScope,
    };
    use std::collections::HashMap;

    fn node(id: &str, path: &str) -> Node {
        Node {
            id: id.to_string(),
            path: path.to_string(),
            name: path.trim_start_matches('/').to_string(),
            parent: Some("/".to_string()),
            node_type: "raisin:Page".to_string(),
            properties: HashMap::new(),
            ..Default::default()
        }
    }

    async fn create(storage: &crate::RocksDBStorage, ws: &str, id: &str) {
        storage
            .nodes()
            .create(
                StorageScope::new("t", "r", "main", ws),
                node(id, &format!("/{id}")),
                CreateNodeOptions {
                    validate_schema: false,
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .expect("create");
    }

    /// `prefix_iterator_cf` does not stop at the end of the prefix, so an
    /// unfiltered COUNT(*) used to count every node in every later workspace
    /// as well. A four-node workspace answered 639 on a small server.
    #[tokio::test]
    async fn count_all_counts_only_the_named_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage = crate::RocksDBStorage::new(tmp.path()).unwrap();
        // Without the branch every `create` below fails `Branch 'main' not
        // found`, which is what had this test red.
        storage
            .branches()
            .create_branch("t", "r", "main", "test", None, None, false, false)
            .await
            .expect("create branch");

        for id in ["a1", "a2"] {
            create(&storage, "aaa", id).await;
        }
        for id in ["b1", "b2", "b3", "b4"] {
            create(&storage, "bbb", id).await;
        }
        // A workspace that sorts AFTER both, so a runaway iterator would reach it.
        for id in ["z1"] {
            create(&storage, "zzz", id).await;
        }

        async fn count(storage: &crate::RocksDBStorage, ws: &str) -> usize {
            storage
                .nodes()
                .count_all(StorageScope::new("t", "r", "main", ws), None)
                .await
                .unwrap()
        }
        assert_eq!(count(&storage, "aaa").await, 2);
        assert_eq!(count(&storage, "bbb").await, 4);
        assert_eq!(count(&storage, "zzz").await, 1);
        assert_eq!(count(&storage, "nothing").await, 0);
    }
}
