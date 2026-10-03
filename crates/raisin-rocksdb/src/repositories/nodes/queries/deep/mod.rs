//! Deep query operations for hierarchical node retrieval
//!
//! This module provides functions for deep queries that traverse the node tree:
//! - Nested structure (HashMap of DeepNodes)
//! - Flat structure (Vec<Node> in fractional index order)
//! - Array structure (Vec<NodeWithChildren>)

mod array;
mod flat;
mod nested;
mod tree_shape;

use super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use std::collections::HashMap;
use tree_shape::TreeShape;

/// Everything a nested/array tree build carries down its recursion.
struct DeepCtx<'a> {
    tenant_id: &'a str,
    repo_id: &'a str,
    branch: &'a str,
    workspace: &'a str,
    max_depth: u32,
    max_revision: Option<&'a HLC>,
    /// The bulk-fetched subtree, by node id.
    nodes_by_id: &'a HashMap<String, Node>,
    shape: &'a TreeShape,
}

impl NodeRepositoryImpl {
    /// Resolve the parent ID for deep query operations.
    ///
    /// For root path, returns "/" (root-level children are indexed with parent_id="/").
    /// For non-root, looks up the parent node to get its ID.
    pub(in crate::repositories::nodes) async fn resolve_parent_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<String> {
        if parent_path == "/" || parent_path.is_empty() {
            Ok("/".to_string())
        } else {
            let parent = self
                .get_by_path_impl(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    parent_path,
                    max_revision,
                )
                .await?
                .ok_or_else(|| {
                    raisin_error::Error::NotFound("Parent node not found".to_string())
                })?;
            Ok(parent.id)
        }
    }

    /// `has_children` for a node the tree build emits at `current_depth`
    /// (its relative depth is `current_depth + 1`), whose children in the bulk
    /// set are `children`.
    ///
    /// The bulk set holds every node down to `max_depth`, so below that depth
    /// "no children in the set" means a leaf and costs nothing. A node whose
    /// own children lie past `max_depth` is on the boundary: the set cannot
    /// answer for it, and the existence probe does.
    fn known_has_children(
        &self,
        ctx: &DeepCtx<'_>,
        node: &Node,
        children: &[String],
        current_depth: u32,
    ) -> Result<bool> {
        if !children.is_empty() {
            return Ok(true);
        }
        let children_beyond_set = u64::from(current_depth) + 2 > u64::from(ctx.max_depth);
        if !children_beyond_set {
            return Ok(false);
        }
        self.probe_has_children(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            &node.id,
            Some(&node.path),
            ctx.max_revision,
        )
    }
}
