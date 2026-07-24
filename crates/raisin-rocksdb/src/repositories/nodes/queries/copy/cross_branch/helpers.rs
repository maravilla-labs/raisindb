//! Parent-id resolution and stale-placement tombstones for cross-branch copy.

use super::super::super::super::helpers::TOMBSTONE;
use super::super::super::super::NodeRepositoryImpl;
use super::parent_path_of;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Resolve a parent path to the parent id used by the ORDERED_CHILDREN
    /// index: `"/"` for root-level nodes, otherwise the parent node's id.
    pub(super) async fn resolve_parent_id_opt(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
    ) -> Result<Option<String>> {
        if parent_path == "/" {
            return Ok(Some("/".to_string()));
        }
        Ok(self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, parent_path, None)
            .await?
            .map(|p| p.id))
    }

    /// Tombstone the stale PATH_INDEX and ORDERED_CHILDREN entries left on the
    /// target branch when a node moved/renamed/reordered on the source since
    /// the last promotion.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn tombstone_stale_placement(
        &self,
        batch: &mut WriteBatch,
        old_dst: &Node,
        new_node: &Node,
        new_parent_id: &str,
        new_label: &str,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        if old_dst.path != new_node.path {
            let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
            let old_path_key = keys::path_index_key_versioned(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old_dst.path,
                revision,
            );
            batch.put_cf(cf_path, old_path_key, TOMBSTONE);
        }

        let old_parent_path = parent_path_of(&old_dst.path);
        let old_parent_id = self
            .resolve_parent_id_opt(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old_parent_path,
            )
            .await?;
        if let Some(old_pid) = old_parent_id {
            if let Some(old_label) = self.get_order_label_for_child(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old_pid,
                &new_node.id,
            )? {
                if old_pid != new_parent_id || old_label != new_label {
                    let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
                    let ordered_key = keys::ordered_child_key_versioned(
                        tenant_id,
                        repo_id,
                        target_branch,
                        workspace,
                        &old_pid,
                        &old_label,
                        revision,
                        &new_node.id,
                    );
                    batch.put_cf(cf_ordered, ordered_key, TOMBSTONE);
                }
            }
        }

        Ok(())
    }
}
