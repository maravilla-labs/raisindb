//! Array deep query: returns Vec<NodeWithChildren> with flexible children field.

use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::{Node, NodeWithChildren};
use std::collections::HashMap;

impl NodeRepositoryImpl {
    /// Get deep children as ordered array with flexible children field
    ///
    /// Each node has a `children` field that is either:
    /// - `ChildrenField::Nodes` - Recursively expanded children (within max_depth)
    /// - `ChildrenField::Names` - Just child names (when max_depth is reached)
    ///
    /// Uses `get_descendants_bulk()` to fetch all nodes at once, then builds the tree
    /// structure from the in-memory data.
    pub(in crate::repositories::nodes) async fn deep_children_array_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<NodeWithChildren>> {
        tracing::debug!(
            "deep_children_array_impl: parent_path='{}', max_depth={}",
            parent_path,
            max_depth
        );

        let parent_id = self
            .resolve_parent_id(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_path,
                max_revision,
            )
            .await?;

        // Bulk fetch all descendants at once
        let all_descendants = self
            .get_descendants_bulk_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_path,
                max_depth,
                max_revision,
            )
            .await?;

        // Build index by node ID for fast lookups
        let nodes_by_id: HashMap<String, Node> = all_descendants
            .into_values()
            .map(|node| (node.id.clone(), node))
            .collect();

        tracing::debug!(
            "deep_children_array_impl: fetched {} nodes, building tree",
            nodes_by_id.len()
        );

        self.build_array_children_from_cache(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &parent_id,
            0,
            max_depth,
            max_revision,
            &nodes_by_id,
        )
        .await
    }

    /// Helper for building NodeWithChildren array from cached nodes + ORDERED_CHILDREN CF
    fn build_array_children_from_cache<'a>(
        &'a self,
        tenant_id: &'a str,
        repo_id: &'a str,
        branch: &'a str,
        workspace: &'a str,
        parent_id: &'a str,
        current_depth: u32,
        max_depth: u32,
        max_revision: Option<&'a HLC>,
        nodes_by_id: &'a HashMap<String, Node>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<NodeWithChildren>>> + Send + 'a>,
    > {
        Box::pin(async move {
            let child_ids = self
                .get_ordered_child_ids(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    parent_id,
                    max_revision,
                )
                .await?;

            let mut result = Vec::with_capacity(child_ids.len());

            for child_id in child_ids {
                if let Some(child) = nodes_by_id.get(&child_id).cloned() {
                    let child_names_list = self
                        .get_ordered_child_ids(
                            tenant_id,
                            repo_id,
                            branch,
                            workspace,
                            &child_id,
                            max_revision,
                        )
                        .await?;

                    let node_with_children = if current_depth >= max_depth {
                        NodeWithChildren {
                            node: child,
                            children: raisin_models::nodes::ChildrenField::Names(child_names_list),
                        }
                    } else {
                        let expanded_children = self
                            .build_array_children_from_cache(
                                tenant_id,
                                repo_id,
                                branch,
                                workspace,
                                &child_id,
                                current_depth + 1,
                                max_depth,
                                max_revision,
                                nodes_by_id,
                            )
                            .await?;

                        NodeWithChildren {
                            node: child,
                            children: raisin_models::nodes::ChildrenField::Nodes(
                                expanded_children.into_iter().map(Box::new).collect(),
                            ),
                        }
                    };

                    result.push(node_with_children);
                }
            }

            Ok(result)
        })
    }
}
