//! Flat deep query: returns Vec<Node> in fractional index order.

use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use std::future::Future;
use std::pin::Pin;

use super::tree_shape::TreeShape;

impl NodeRepositoryImpl {
    /// Get deep children in flat structure (Vec of Nodes in fractional index order)
    ///
    /// Uses ORDERED_CHILDREN CF traversal to collect IDs in order, then fetches
    /// nodes individually. This preserves fractional index order which is important
    /// for REST endpoints.
    pub(in crate::repositories::nodes) async fn deep_children_flat_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<Node>> {
        tracing::debug!(
            "deep_children_flat_impl: parent_path='{}', max_depth={}",
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

        // The subtree's shape from PATH_INDEX alone (no node blob read). The
        // walk below emits children down to relative depth `max_depth + 1`,
        // so the shape is taken one level deeper than `max_depth`.
        let (_, entries) = self.descendant_index_entries(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            max_depth.saturating_add(1),
            max_revision,
        )?;
        let shape = TreeShape::new(
            entries
                .iter()
                .map(|(id, (path, _))| (id.as_str(), path.as_str())),
        );

        // Collect IDs in editorial order, depth first
        let mut ordered_ids = Vec::new();
        let walk = FlatWalk {
            tenant_id,
            repo_id,
            branch,
            workspace,
            max_depth,
            max_revision,
            shape: &shape,
        };
        self.collect_ordered_descendant_ids(
            &walk,
            &parent_id,
            shape.children_of_path(parent_path),
            0,
            &mut ordered_ids,
        )
        .await?;

        tracing::debug!(
            "deep_children_flat_impl: collected {} ordered IDs, fetching nodes",
            ordered_ids.len()
        );

        // Resolve the target revision for node lookups
        let target_revision = if let Some(rev) = max_revision {
            *rev
        } else if let Some(head) = self
            .resolve_head_revision(tenant_id, repo_id, branch)
            .await?
        {
            head
        } else {
            return Ok(Vec::new());
        };

        // Fetch nodes by ID preserving order
        let mut result = Vec::with_capacity(ordered_ids.len());
        for id in ordered_ids {
            if let Some(node) = self
                .get_at_revision_impl(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &id,
                    &target_revision,
                    false, // populate_has_children - not needed for flat list
                )
                .await?
            {
                result.push(node);
            }
        }

        tracing::debug!(
            "deep_children_flat_impl: returning {} descendants",
            result.len()
        );

        Ok(result)
    }

    /// Collect descendant ids in editorial order, depth first (pre-order).
    ///
    /// `child_ids` are `parent_id`'s children from the subtree shape. A node
    /// the shape shows to be a leaf is never scanned, and a parent with one
    /// child needs no ordering scan either; see `tree_shape`.
    fn collect_ordered_descendant_ids<'a>(
        &'a self,
        walk: &'a FlatWalk<'a>,
        parent_id: &'a str,
        child_ids: &'a [String],
        current_depth: u32,
        result: &'a mut Vec<String>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if current_depth > walk.max_depth {
                return Ok(());
            }

            let child_ids = self
                .order_child_ids(
                    walk.tenant_id,
                    walk.repo_id,
                    walk.branch,
                    walk.workspace,
                    parent_id,
                    child_ids,
                    walk.max_revision,
                )
                .await?;

            for child_id in child_ids {
                result.push(child_id.clone());

                let grandchildren = walk.shape.children_of(&child_id);
                if !grandchildren.is_empty() {
                    self.collect_ordered_descendant_ids(
                        walk,
                        &child_id,
                        grandchildren,
                        current_depth + 1,
                        result,
                    )
                    .await?;
                }
            }

            Ok(())
        })
    }
}

/// What the flat walk carries down its recursion.
struct FlatWalk<'a> {
    tenant_id: &'a str,
    repo_id: &'a str,
    branch: &'a str,
    workspace: &'a str,
    max_depth: u32,
    max_revision: Option<&'a HLC>,
    shape: &'a TreeShape,
}
