//! Revision-based deep tree query operations.
//!
//! Provides methods for querying deep tree structures at specific revisions
//! using Merkle-like tree snapshots. Supports nested, flat, and array formats.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_storage::{scope::RepoScope, NodeRepository, Storage, TreeRepository};

use super::super::NodeService;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage> NodeService<S> {
    /// Helper: Deep query with nested structure at specific revision
    pub(in crate::services::node_service) async fn deep_children_nested_at_revision(
        &self,
        parent_path: &str,
        max_depth: u32,
        revision: &HLC,
    ) -> Result<std::collections::HashMap<String, models::nodes::DeepNode>> {
        use std::collections::HashMap;

        // Get root tree for this revision
        let root_tree_id = self
            .storage
            .trees()
            .get_root_tree_id(RepoScope::new(&self.tenant_id, &self.repo_id), revision)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("No tree found at revision {}", revision))
            })?;

        // Find starting tree ID for parent path
        let start_tree_id = if parent_path == "/" || parent_path.is_empty() {
            root_tree_id
        } else {
            self.find_children_tree_id_for_path(&root_tree_id, parent_path, revision)
                .await?
                .ok_or_else(|| {
                    raisin_error::Error::NotFound(format!(
                        "Parent path '{}' has no children at revision {}",
                        parent_path, revision
                    ))
                })?
        };

        let mut result = HashMap::new();
        self.collect_deep_nested_from_tree(&start_tree_id, "", max_depth, 0, revision, &mut result)
            .await?;

        Ok(result)
    }

    /// Helper: Iteratively collect nodes in nested structure
    async fn collect_deep_nested_from_tree(
        &self,
        tree_id: &[u8; 32],
        parent_path: &str,
        max_depth: u32,
        _current_depth: u32,
        revision: &HLC,
        result: &mut std::collections::HashMap<String, models::nodes::DeepNode>,
    ) -> Result<()> {
        // Iterative approach using work queue
        let mut queue: Vec<([u8; 32], String, u32)> = vec![(*tree_id, parent_path.to_string(), 0)];

        // First pass: collect all nodes with their paths
        let mut nodes_by_path: std::collections::HashMap<String, models::nodes::Node> =
            std::collections::HashMap::new();
        let mut children_map: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();

        while let Some((tid, path_prefix, depth)) = queue.pop() {
            if depth >= max_depth {
                continue;
            }

            let entries = self
                .storage
                .trees()
                .iter_tree(
                    RepoScope::new(&self.tenant_id, &self.repo_id),
                    &tid,
                    None,
                    10000,
                )
                .await?;

            for entry in entries {
                if let Some(node) = self
                    .storage
                    .nodes()
                    .get(self.scope(), &entry.node_id, Some(revision))
                    .await?
                {
                    let full_path = if path_prefix.is_empty() {
                        format!("/{}", node.name)
                    } else {
                        format!("{}/{}", path_prefix, node.name)
                    };

                    nodes_by_path.insert(full_path.clone(), node);
                    children_map
                        .entry(path_prefix.clone())
                        .or_default()
                        .push(full_path.clone());

                    if let Some(children_tree_id) = entry.children_tree_id {
                        queue.push((children_tree_id, full_path, depth + 1));
                    }
                }
            }
        }

        // Second pass: build nested structure bottom-up
        let mut paths: Vec<String> = nodes_by_path.keys().cloned().collect();
        paths.sort_by(|a, b| {
            let depth_a = a.matches('/').count();
            let depth_b = b.matches('/').count();
            depth_b.cmp(&depth_a)
        });

        for path in paths {
            if let Some(node) = nodes_by_path.get(&path) {
                let children = if let Some(child_paths) = children_map.get(&path) {
                    child_paths
                        .iter()
                        .filter_map(|cp| result.get(cp).cloned())
                        .map(|dn| (dn.node.name.clone(), dn))
                        .collect()
                } else {
                    std::collections::HashMap::new()
                };

                result.insert(
                    path.clone(),
                    models::nodes::DeepNode {
                        node: node.clone(),
                        children,
                    },
                );
            }
        }

        Ok(())
    }

    /// Helper: Deep query with flat structure at specific revision
    pub(in crate::services::node_service) async fn deep_children_flat_at_revision(
        &self,
        parent_path: &str,
        max_depth: u32,
        revision: &HLC,
    ) -> Result<std::collections::HashMap<String, models::nodes::Node>> {
        use std::collections::HashMap;

        // Get root tree for this revision
        let root_tree_id = self
            .storage
            .trees()
            .get_root_tree_id(RepoScope::new(&self.tenant_id, &self.repo_id), revision)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("No tree found at revision {}", revision))
            })?;

        // Find starting tree ID for parent path
        let start_tree_id = if parent_path == "/" || parent_path.is_empty() {
            root_tree_id
        } else {
            self.find_children_tree_id_for_path(&root_tree_id, parent_path, revision)
                .await?
                .ok_or_else(|| {
                    raisin_error::Error::NotFound(format!(
                        "Parent path '{}' has no children at revision {}",
                        parent_path, revision
                    ))
                })?
        };

        let mut result = HashMap::new();
        self.collect_deep_flat_from_tree(&start_tree_id, "", max_depth, 0, revision, &mut result)
            .await?;

        Ok(result)
    }

    /// Helper: Iteratively collect nodes in flat structure
    async fn collect_deep_flat_from_tree(
        &self,
        tree_id: &[u8; 32],
        parent_path: &str,
        max_depth: u32,
        _current_depth: u32,
        revision: &HLC,
        result: &mut std::collections::HashMap<String, models::nodes::Node>,
    ) -> Result<()> {
        // Iterative approach using work queue
        let mut queue: Vec<([u8; 32], String, u32)> = vec![(*tree_id, parent_path.to_string(), 0)];

        while let Some((tid, path_prefix, depth)) = queue.pop() {
            if depth >= max_depth {
                continue;
            }

            let entries = self
                .storage
                .trees()
                .iter_tree(
                    RepoScope::new(&self.tenant_id, &self.repo_id),
                    &tid,
                    None,
                    10000,
                )
                .await?;

            for entry in entries {
                if let Some(node) = self
                    .storage
                    .nodes()
                    .get(self.scope(), &entry.node_id, Some(revision))
                    .await?
                {
                    let full_path = if path_prefix.is_empty() {
                        format!("/{}", node.name)
                    } else {
                        format!("{}/{}", path_prefix, node.name)
                    };

                    result.insert(full_path.clone(), node);

                    if let Some(children_tree_id) = entry.children_tree_id {
                        queue.push((children_tree_id, full_path, depth + 1));
                    }
                }
            }
        }

        Ok(())
    }

    /// Helper: Deep query with array format at specific revision (iterative)
    pub(in crate::services::node_service) async fn deep_children_array_at_revision(
        &self,
        parent_path: &str,
        max_depth: u32,
        revision: &HLC,
    ) -> Result<Vec<models::nodes::NodeWithChildren>> {
        // Get root tree for this revision
        let root_tree_id = self
            .storage
            .trees()
            .get_root_tree_id(RepoScope::new(&self.tenant_id, &self.repo_id), revision)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("No tree found at revision {}", revision))
            })?;

        // Find starting tree ID for parent path
        let start_tree_id = if parent_path == "/" || parent_path.is_empty() {
            root_tree_id
        } else {
            self.find_children_tree_id_for_path(&root_tree_id, parent_path, revision)
                .await?
                .ok_or_else(|| {
                    raisin_error::Error::NotFound(format!(
                        "Parent path '{}' has no children at revision {}",
                        parent_path, revision
                    ))
                })?
        };

        // Build using iterative approach with work queue
        // Store (tree_id, path_prefix, depth)
        let mut queue: Vec<([u8; 32], String, u32)> = vec![(start_tree_id, String::new(), 0)];

        // Collect all nodes with their paths first
        let mut nodes_by_path: std::collections::HashMap<
            String,
            (models::nodes::Node, Option<[u8; 32]>),
        > = std::collections::HashMap::new();

        while let Some((tree_id, path_prefix, depth)) = queue.pop() {
            if depth >= max_depth {
                continue;
            }

            // Get all entries from this tree
            let entries = self
                .storage
                .trees()
                .iter_tree(
                    RepoScope::new(&self.tenant_id, &self.repo_id),
                    &tree_id,
                    None,
                    10000,
                )
                .await?;

            for entry in entries {
                // Get node from NODES CF at this revision
                if let Some(node) = self
                    .storage
                    .nodes()
                    .get(self.scope(), &entry.node_id, Some(revision))
                    .await?
                {
                    let node_path = if path_prefix.is_empty() {
                        node.name.clone()
                    } else {
                        format!("{}/{}", path_prefix, node.name)
                    };

                    nodes_by_path.insert(node_path.clone(), (node, entry.children_tree_id));

                    // Queue children for processing
                    if let Some(children_tree_id) = entry.children_tree_id {
                        queue.push((children_tree_id, node_path, depth + 1));
                    }
                }
            }
        }

        // Now build the nested structure bottom-up
        // Sort paths by depth (deepest first) so we build from leaves up
        let mut paths: Vec<String> = nodes_by_path.keys().cloned().collect();
        paths.sort_by(|a, b| {
            let depth_a = a.matches('/').count();
            let depth_b = b.matches('/').count();
            depth_b.cmp(&depth_a) // Reverse order - deepest first
        });

        // Build NodeWithChildren for each node
        let mut built_nodes: std::collections::HashMap<String, models::nodes::NodeWithChildren> =
            std::collections::HashMap::new();

        for path in paths {
            if let Some((node, _children_tree_id)) = nodes_by_path.get(&path) {
                // Collect children that have this path as their parent
                let child_prefix = format!("{}/", path);
                let mut children_vec: Vec<models::nodes::NodeWithChildren> = built_nodes
                    .iter()
                    .filter(|(child_path, _)| {
                        child_path.starts_with(&child_prefix)
                            && child_path[child_prefix.len()..].find('/').is_none()
                    })
                    .map(|(_, child_node)| child_node.clone())
                    .collect();

                // Sort children by name for consistent ordering
                children_vec.sort_by(|a, b| a.node.name.cmp(&b.node.name));

                // Clone node and clear its children field to avoid duplicate serialization
                let mut node_clean = node.clone();
                node_clean.children.clear();

                // Use NodeWithChildren::new() to properly handle the node's children field
                let node_with_children = if children_vec.is_empty() {
                    models::nodes::NodeWithChildren::new(node_clean)
                } else {
                    models::nodes::NodeWithChildren::new(node_clean).with_children(children_vec)
                };

                built_nodes.insert(path.clone(), node_with_children);
            }
        }

        // Collect root-level nodes (no '/' in path)
        let mut result: Vec<models::nodes::NodeWithChildren> = built_nodes
            .into_iter()
            .filter(|(path, _)| !path.contains('/'))
            .map(|(_, node)| node)
            .collect();

        // Sort by name for consistent ordering
        result.sort_by(|a, b| a.node.name.cmp(&b.node.name));

        Ok(result)
    }
}
