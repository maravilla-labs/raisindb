//! Array deep query: returns Vec<NodeWithChildren> with flexible children field.

use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::{ChildrenField, Node, NodeWithChildren};
use std::collections::HashMap;

use super::tree_shape::TreeShape;
use super::DeepCtx;

/// Boxed future for recursive async tree building.
type ArrayFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<NodeWithChildren>>> + Send + 'a>>;

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
        let shape = TreeShape::new(
            nodes_by_id
                .values()
                .map(|node| (node.id.as_str(), node.path.as_str())),
        );

        tracing::debug!(
            "deep_children_array_impl: fetched {} nodes, building tree",
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
        self.build_array_children(&ctx, &parent_id, shape.children_of_path(parent_path), 0)
            .await
    }

    /// `NodeWithChildren` entries for the children of `parent_id` (whose
    /// children, unordered, are `child_ids`) from the bulk-fetched nodes.
    ///
    /// Each child is scanned for at most once: an expanded child's own
    /// children come from the bulk set (and are ordered by the recursion), and
    /// only a child at `max_depth` — reported by name — reads its child list
    /// from `ORDERED_CHILDREN`. The old build read that list for EVERY child
    /// and then threw it away whenever it expanded the child.
    fn build_array_children<'a>(
        &'a self,
        ctx: &'a DeepCtx<'a>,
        parent_id: &'a str,
        child_ids: &'a [String],
        current_depth: u32,
    ) -> ArrayFuture<'a> {
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

            let mut result = Vec::with_capacity(child_ids.len());

            for child_id in child_ids {
                if let Some(mut child) = ctx.nodes_by_id.get(&child_id).cloned() {
                    let grandchildren = ctx.shape.children_of(&child_id);

                    let node_with_children = if current_depth >= ctx.max_depth {
                        let child_names_list = self
                            .get_ordered_child_ids(
                                ctx.tenant_id,
                                ctx.repo_id,
                                ctx.branch,
                                ctx.workspace,
                                &child_id,
                                ctx.max_revision,
                            )
                            .await?;
                        child.has_children = Some(!child_names_list.is_empty());
                        NodeWithChildren {
                            node: child,
                            children: ChildrenField::Names(child_names_list),
                        }
                    } else {
                        child.has_children = Some(self.known_has_children(
                            ctx,
                            &child,
                            grandchildren,
                            current_depth,
                        )?);
                        let expanded_children = self
                            .build_array_children(ctx, &child_id, grandchildren, current_depth + 1)
                            .await?;

                        NodeWithChildren {
                            node: child,
                            children: ChildrenField::Nodes(
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
