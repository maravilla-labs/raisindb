//! Nested deep query: returns HashMap<String, DeepNode> tree structure.

use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::{DeepNode, Node};
use std::collections::HashMap;

/// Boxed future for recursive async tree building.
type DeepNodeFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<HashMap<String, DeepNode>>> + Send + 'a>,
>;

impl NodeRepositoryImpl {
    pub(in crate::repositories::nodes) async fn deep_children_nested_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> Result<HashMap<String, DeepNode>> {
        tracing::debug!(
            "deep_children_nested_impl: parent_path='{}', max_depth={}",
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
            "deep_children_nested_impl: fetched {} nodes, building tree",
            nodes_by_id.len()
        );

        // Build the nested structure from cached nodes + ORDERED_CHILDREN CF for ordering
        self.build_nested_children_from_cache(
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

    /// Helper for building nested DeepNode children from cached nodes + ORDERED_CHILDREN CF
    fn build_nested_children_from_cache<'a>(
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
    ) -> DeepNodeFuture<'a> {
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

            let mut result = HashMap::with_capacity(child_ids.len());

            for child_id in child_ids {
                if let Some(child) = nodes_by_id.get(&child_id).cloned() {
                    let child_name = child.name.clone();

                    let deep_node = if current_depth >= max_depth {
                        DeepNode::new(child)
                    } else {
                        let nested_children = self
                            .build_nested_children_from_cache(
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

                        DeepNode {
                            node: child,
                            children: nested_children,
                        }
                    };

                    result.insert(child_name, deep_node);
                }
            }

            Ok(result)
        })
    }
}
