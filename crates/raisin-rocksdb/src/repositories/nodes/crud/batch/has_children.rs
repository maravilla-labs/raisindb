//! Has-children population helper.
//!
//! Contains `populate_node_has_children` which uses a boxed future
//! to break recursive async function cycles.

use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// Populate has_children field for a node
    ///
    /// This helper uses the MVCC-aware existence probe
    /// ([`Self::probe_has_children`]) to determine whether a node has children
    /// at a specific revision (or HEAD if None).
    ///
    /// Returns a boxed future to break recursive async function cycles.
    pub(in super::super::super) fn populate_node_has_children<'a>(
        &'a self,
        tenant_id: &'a str,
        repo_id: &'a str,
        branch: &'a str,
        workspace: &'a str,
        node: &'a mut Node,
        max_revision: Option<&'a HLC>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // The node is already in hand, so its path tells the probe whether
            // it is the root — no reload.
            let has_children = self.probe_has_children(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node.id,
                Some(&node.path),
                max_revision,
            )?;

            node.has_children = Some(has_children);
            Ok(())
        })
    }
}
