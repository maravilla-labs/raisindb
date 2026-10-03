//! Nested deep query: returns HashMap<String, DeepNode> tree structure.

use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::{DeepNode, Node};
use std::collections::HashMap;

use super::tree_shape::TreeShape;
use super::DeepCtx;

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
        let shape = TreeShape::new(
            nodes_by_id
                .values()
                .map(|node| (node.id.as_str(), node.path.as_str())),
        );

        tracing::debug!(
            "deep_children_nested_impl: fetched {} nodes, building tree",
            nodes_by_id.len()
        );

        let ctx = DeepCtx {
            tenant_id,
            repo_id,
            branch,
            workspace,
            max_depth,
            max_revision,
            nodes_by_id: &nodes_by_id,
            shape: &shape,
        };
        self.build_nested_children(&ctx, &parent_id, shape.children_of_path(parent_path), 0)
            .await
    }

    /// Nested `DeepNode` children of `parent_id` (whose children, unordered,
    /// are `child_ids`) from the bulk-fetched nodes. Only a parent with two or
    /// more children scans `ORDERED_CHILDREN`; see `tree_shape`.
    fn build_nested_children<'a>(
        &'a self,
        ctx: &'a DeepCtx<'a>,
        parent_id: &'a str,
        child_ids: &'a [String],
        current_depth: u32,
    ) -> DeepNodeFuture<'a> {
        Box::pin(async move {
            let child_ids = self
                .order_child_ids(
                    ctx.tenant_id,
                    ctx.repo_id,
                    ctx.branch,
                    ctx.workspace,
                    parent_id,
                    child_ids,
                    ctx.max_revision,
                )
                .await?;

            let mut result = HashMap::with_capacity(child_ids.len());

            for child_id in child_ids {
                if let Some(mut child) = ctx.nodes_by_id.get(&child_id).cloned() {
                    let grandchildren = ctx.shape.children_of(&child_id);
                    child.has_children =
                        Some(self.known_has_children(ctx, &child, grandchildren, current_depth)?);
                    let child_name = child.name.clone();

                    let deep_node = if current_depth >= ctx.max_depth {
                        DeepNode::new(child)
                    } else {
                        let nested_children = self
                            .build_nested_children(ctx, &child_id, grandchildren, current_depth + 1)
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
