//! Flat deep query: returns Vec<Node> in fractional index order.

use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use std::future::Future;
use std::pin::Pin;

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

        // Collect IDs in fractional index order (depth-first via ORDERED_CHILDREN)
        let mut ordered_ids = Vec::new();
        self.collect_ordered_descendant_ids(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &parent_id,
            0,
            max_depth,
            max_revision,
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

    /// Collect descendant IDs in fractional index order (depth-first via ORDERED_CHILDREN)
    fn collect_ordered_descendant_ids<'a>(
        &'a self,
        tenant_id: &'a str,
        repo_id: &'a str,
        branch: &'a str,
        workspace: &'a str,
        parent_id: &'a str,
        current_depth: u32,
        max_depth: u32,
        max_revision: Option<&'a HLC>,
        result: &'a mut Vec<String>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if current_depth > max_depth {
                return Ok(());
            }

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

            for child_id in child_ids {
                result.push(child_id.clone());

                self.collect_ordered_descendant_ids(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &child_id,
                    current_depth + 1,
                    max_depth,
                    max_revision,
                    result,
                )
                .await?;
            }

            Ok(())
        })
    }
}
