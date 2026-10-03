//! Node listing operations
//!
//! This module provides functions for listing nodes by various criteria:
//! - List by type
//! - List by parent
//! - List root nodes
//! - List children
//! - Check if node has children

use super::super::ordering::OrderedScanStart;
use super::super::storage_node::PropertiesMode;
use super::super::NodeRepositoryImpl;
use crate::repositories::property_index::reader::PropertyIndexReader;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// List nodes by type using __node_type pseudo-property index
    pub(in crate::repositories::nodes) async fn list_by_type_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<Node>> {
        // The __node_type pseudo-property, through the one revision-bounded
        // PROPERTY_INDEX reader: as of `max_revision`, else the branch HEAD.
        let node_ids = PropertyIndexReader::new(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            "__node_type",
            false,
            max_revision,
        )?
        .find(&PropertyValue::String(node_type.to_string()), None)?;

        // Fetch actual nodes
        let mut nodes = Vec::new();
        for node_id in node_ids {
            // Public API - populate has_children for frontend display
            let node_opt = match max_revision {
                Some(rev) => {
                    self.get_at_revision_impl(
                        tenant_id, repo_id, branch, workspace, &node_id, rev, true,
                    )
                    .await?
                }
                None => {
                    self.get_impl(tenant_id, repo_id, branch, workspace, &node_id, true)
                        .await?
                }
            };
            // The index yields candidates. A pseudo-property has no residual
            // filter downstream, so an orphan `__node_type` entry left by an
            // old writer would otherwise list a node whose type has changed.
            if let Some(node) = node_opt.filter(|node| node.node_type == node_type) {
                nodes.push(node);
            }
        }

        Ok(nodes)
    }

    /// List children using ORDERED_CHILDREN index
    pub(in crate::repositories::nodes) async fn list_by_parent_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent: &str,
        max_revision: Option<&HLC>,
        populate_has_children: bool,
    ) -> Result<Vec<Node>> {
        Ok(self
            .list_by_parent_paged_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent,
                None,
                None,
                false,
                max_revision,
                populate_has_children,
                PropertiesMode::Load,
            )
            .await?
            .into_iter()
            .map(|(node, _label)| node)
            .collect())
    }

    /// List children in editorial order, keyset-paginated, returning each
    /// child's order label alongside the node.
    ///
    /// This is the single implementation behind [`Self::list_by_parent_impl`]
    /// (which discards the labels and passes no cursor). The label is needed by
    /// callers that expose editorial order — the `__order` SQL column and the
    /// cursor-paginated child listing — and comes for free, since it is already
    /// part of the scanned index key.
    ///
    /// See [`NodeRepository::list_ordered_children_page`] for cursor semantics.
    ///
    /// [`NodeRepository::list_ordered_children_page`]: raisin_storage::NodeRepository::list_ordered_children_page
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) async fn list_by_parent_paged_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent: &str,
        after_label: Option<&str>,
        limit: Option<usize>,
        descending: bool,
        max_revision: Option<&HLC>,
        populate_has_children: bool,
        properties: PropertiesMode,
    ) -> Result<Vec<(Node, String)>> {
        // Special case: "/" is the parent NAME for root-level nodes,
        // For root nodes, we use "/" itself as the parent_id
        let parent_id = if parent == "/" {
            "/".to_string()
        } else {
            // For non-root parents, parent is already the ID
            parent.to_string()
        };

        tracing::debug!(
            "list_by_parent_paged_impl: tenant={}, repo={}, branch={}, workspace={}, parent={}, \
             after_label={:?}, limit={:?}, descending={}, max_revision={:?}",
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_id,
            after_label,
            limit,
            descending,
            max_revision
        );

        let entries = self.list_ordered_children_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &parent_id,
            match after_label {
                Some(label) => OrderedScanStart::After(label),
                None => OrderedScanStart::Beginning,
            },
            limit,
            descending,
            max_revision,
        )?;

        tracing::debug!(
            "list_by_parent_paged_impl: got {} child entries from ordered index",
            entries.len()
        );

        // Fetch nodes in order, keeping each one's order label.
        //
        // An entry whose node cannot be loaded is skipped: the index can
        // legitimately outlive the node (concurrent delete, or a not-yet-visible
        // revision). Note this means a page can return fewer rows than `limit`
        // without being the last page — callers must drive their cursor from the
        // last returned label, not from the row count.
        let mut result = Vec::with_capacity(entries.len());
        for entry in entries {
            let node_opt = match max_revision {
                Some(rev) => {
                    self.get_at_revision_impl_as(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &entry.child_id,
                        rev,
                        populate_has_children,
                        properties,
                    )
                    .await?
                }
                None => {
                    self.get_impl_as(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &entry.child_id,
                        populate_has_children,
                        properties,
                    )
                    .await?
                }
            };
            if let Some(node) = node_opt {
                result.push((node, entry.order_label));
            }
        }

        Ok(result)
    }

    /// List root nodes (nodes whose parent is "/" or root itself)
    pub(in crate::repositories::nodes) async fn list_root_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<Node>> {
        // Public API - populate has_children for frontend display
        self.list_by_parent_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            "/",
            max_revision,
            true,
        )
        .await
    }

    /// List children by parent path
    pub(in crate::repositories::nodes) async fn list_children_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<Node>> {
        // Special case: root path "/" uses "/" as parent_id in ORDERED_CHILDREN index
        // This matches the logic in add_impl/update_impl where root-level nodes are indexed with parent_id = "/"
        if parent_path == "/" {
            return self
                .list_by_parent_impl(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "/",
                    max_revision,
                    true,
                )
                .await;
        }

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
            .ok_or_else(|| raisin_error::Error::NotFound("Parent node not found".to_string()))?;

        // Public API - populate has_children for frontend display
        self.list_by_parent_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &parent.id,
            max_revision,
            true,
        )
        .await
    }

    /// Check if node has children
    ///
    /// An existence probe over the ORDERED_CHILDREN index that stops at the
    /// first live child; see [`Self::probe_has_children`]. Pass the node's path
    /// when the caller has it, so the root check needs no lookup.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) async fn has_children_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        node_path: Option<&str>,
        max_revision: Option<&HLC>,
    ) -> Result<bool> {
        self.probe_has_children(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            node_path,
            max_revision,
        )
    }
}
