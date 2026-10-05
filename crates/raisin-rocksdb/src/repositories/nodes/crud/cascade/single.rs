//! Single node deletion without cascade.

use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;

impl NodeRepositoryImpl {
    /// Delete a node without cascade (fails if has children)
    ///
    /// This is used when cascade=false in DeleteNodeOptions. It checks for
    /// children and fails if any exist, preventing orphaned nodes.
    ///
    /// # Arguments
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context for the operation
    /// * `node_id` - The ID of the node to delete
    /// * `check_has_children` - Whether to check for children before deleting
    ///
    /// # Returns
    /// * `Ok(true)` if node was deleted
    /// * `Ok(false)` if node didn't exist
    /// * `Err(Error::Validation)` if node has children and check_has_children=true
    /// * `Err` if deletion failed
    pub(in super::super::super) async fn delete_without_cascade(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        check_has_children: bool,
        attribution: crate::repositories::nodes::WriteAttribution<'_>,
    ) -> Result<bool> {
        use raisin_error::Error;

        // Check if node exists
        let node = match self
            .get_impl(tenant_id, repo_id, branch, workspace, node_id, false)
            .await?
        {
            Some(node) => node,
            None => return Ok(false),
        };

        // Check for children if requested, at HEAD. One existence probe: this
        // used to load (and has_children-probe) every child just to test the
        // list for emptiness.
        if check_has_children
            && self.probe_has_children(
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_id,
                Some(&node.path),
                None,
            )?
        {
            return Err(Error::Validation(format!(
                "Cannot delete node '{}': it has children. Enable cascade to delete descendants.",
                node_id
            )));
        }

        // Delete the node itself using the standard delete_impl
        self.delete_impl(tenant_id, repo_id, branch, workspace, node_id, attribution)
            .await
    }
}
