//! Full delete-tombstone set for target-branch nodes pruned by delete_missing.

use super::super::super::super::helpers::{hash_property_value, TOMBSTONE};
use super::super::super::super::NodeRepositoryImpl;
use super::parent_path_of;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{CrossBranchNodeChange, NodeChangeInfo};
use rocksdb::WriteBatch;
use std::collections::HashSet;

impl NodeRepositoryImpl {
    /// Add the full tombstone set for one pruned target-branch node —
    /// mirrors `delete_impl` (node blob, path, property/system-property,
    /// unique, reference, relation, translation, ordered-children entries),
    /// but writes into the shared cross-branch batch instead of committing.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn add_cross_branch_prune_to_batch(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let cf_reference = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // Node blob tombstone
        let node_key = keys::node_key_versioned(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.id,
            revision,
        );
        batch.put_cf(cf_nodes, node_key, TOMBSTONE);

        // Path index tombstone
        let path_key = keys::path_index_key_versioned(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.path,
            revision,
        );
        batch.put_cf(cf_path, path_key, TOMBSTONE);

        // Property index tombstones (user properties + __node_type)
        let is_published = node.published_at.is_some();
        for (prop_name, prop_value) in &node.properties {
            let value_hash = hash_property_value(prop_value);
            let prop_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                prop_name,
                &value_hash,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, prop_key, TOMBSTONE);
        }
        let node_type_key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            "__node_type",
            &node.node_type,
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, node_type_key, TOMBSTONE);

        // System field indexes (__name, __archetype, __created_*, __updated_*)
        self.add_field_tombstones_to_batch(
            batch,
            cf_property,
            node,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.id,
            revision,
            is_published,
        );

        // Unique index tombstones (release unique values)
        self.add_unique_tombstones_to_batch(
            batch,
            node,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            revision,
        )
        .await?;

        // Reference index tombstones (forward + reverse)
        self.add_reference_tombstones_to_batch(
            batch,
            cf_reference,
            node,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.id,
            revision,
            is_published,
        );

        // Relation index tombstones (outgoing + incoming)
        self.add_relation_tombstones_to_batch(
            batch,
            cf_relation,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.id,
            revision,
        )?;

        // Translation tombstones
        self.add_translation_tombstones_to_batch(
            batch,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.id,
            revision,
        )?;

        // Ordered-children tombstone (resolve the parent id by path — the
        // node's `parent` field holds the parent NAME, not its id)
        let parent_path = parent_path_of(&node.path);
        if let Some(parent_id) = self
            .resolve_parent_id_opt(tenant_id, repo_id, target_branch, workspace, &parent_path)
            .await?
        {
            if let Some(label) = self.get_order_label_for_child(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &parent_id,
                &node.id,
            )? {
                let ordered_key = keys::ordered_child_key_versioned(
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    &parent_id,
                    &label,
                    revision,
                    &node.id,
                );
                batch.put_cf(cf_ordered, ordered_key, TOMBSTONE);
            }
        }

        Ok(())
    }
}

impl NodeRepositoryImpl {
    /// STEP 4 of `copy_nodes_across_branches_impl`: tombstone every
    /// target-branch node under the copied roots that no longer exists in the
    /// copied source set. Returns the pruned node ids.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn prune_missing_targets(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
        root_ctxs: &[super::RootContext],
        src_ids: &HashSet<String>,
        changes: &mut Vec<CrossBranchNodeChange>,
        change_infos: &mut Vec<NodeChangeInfo>,
    ) -> Result<HashSet<String>> {
        let mut deleted_ids: HashSet<String> = HashSet::new();
        for rc in root_ctxs {
            // The pre-copy target tree under the same root id (the batch
            // is not committed yet, so this sees the previous state).
            if self
                .get_impl(
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    &rc.node.id,
                    false,
                )
                .await?
                .is_none()
            {
                continue;
            }
            let dst_set = self.scan_descendants_ordered_impl(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &rc.node.id,
                None,
            )?;
            for (dst_node, _) in dst_set {
                if src_ids.contains(&dst_node.id) || !deleted_ids.insert(dst_node.id.clone()) {
                    continue;
                }
                self.add_cross_branch_prune_to_batch(
                    batch,
                    &dst_node,
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    revision,
                )
                .await?;
                changes.push(CrossBranchNodeChange {
                    node_id: dst_node.id.clone(),
                    path: dst_node.path.clone(),
                    node_type: dst_node.node_type.clone(),
                    operation: ChangeOperation::Deleted,
                });
                change_infos.push(NodeChangeInfo {
                    node_id: dst_node.id.clone(),
                    workspace: workspace.to_string(),
                    operation: ChangeOperation::Deleted,
                    translation_locale: None,
                });
            }
        }

        Ok(deleted_ids)
    }
}
