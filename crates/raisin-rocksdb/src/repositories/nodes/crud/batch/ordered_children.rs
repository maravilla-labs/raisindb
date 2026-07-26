//! Ordered children batch operations.
//!
//! Contains `add_ordered_children_to_batch` which manages fractional index
//! labels in the ORDERED_CHILDREN column family during node creation/update.

use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Maintain the `ORDERED_CHILDREN` entry for a created-or-updated node.
    ///
    /// On update the node's existing label is preserved; on insert a new one is
    /// appended. Either way the resulting label is stamped onto
    /// `node.order_key`, so the node record and the index agree — which means
    /// this must be called BEFORE the node blob is serialized into the batch.
    pub(crate) async fn add_ordered_children_to_batch(
        &self,
        batch: &mut WriteBatch,
        node: &mut Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let order_start = std::time::Instant::now();

        // Get column family handle
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // ========== SUBSTEP 1: Parent lookup ==========
        let substep_start = std::time::Instant::now();

        // Get parent ID - node.parent contains parent NAME, but we need parent ID for the index
        let parent_id_for_index = if let Some(parent_path) = node.parent_path() {
            if parent_path == "/" {
                // Special case: root-level nodes use "/" as parent_id
                Some("/".to_string())
            } else {
                // Get parent node to find its ID (always use HEAD for write operations)
                self.get_by_path_impl(tenant_id, repo_id, branch, workspace, &parent_path, None)
                    .await?
                    .map(|p| p.id)
            }
        } else {
            None
        };

        let parent_lookup_time = substep_start.elapsed().as_micros();

        if let Some(ref parent_id) = parent_id_for_index {
            // ========== SUBSTEP 2: Existence check ==========
            let substep_start = std::time::Instant::now();

            // OPTIMIZATION: Check if node exists at HEAD before scanning for existing label
            // This avoids O(n) scan for brand new nodes
            let is_update = self
                .get_latest_revision_for_node(tenant_id, repo_id, branch, workspace, &node.id)?
                .is_some();

            let existence_check_time = substep_start.elapsed().as_micros();

            // ========== SUBSTEP 3: Order label calculation ==========
            let substep_start = std::time::Instant::now();
            let mut get_existing_time = 0u128;
            let mut get_last_time = 0u128;

            let (order_label, is_new_child) = if is_update {
                // Node exists - check for existing order label (may scan, but only for updates)
                let t = std::time::Instant::now();
                let existing_label = self.get_order_label_for_child(
                    tenant_id, repo_id, branch, workspace, parent_id, &node.id,
                )?;
                get_existing_time = t.elapsed().as_micros();

                if let Some(existing) = existing_label {
                    // Preserve existing order label
                    (existing, false)
                } else {
                    // Update without previous order (rare edge case)
                    let t = std::time::Instant::now();
                    let new_label = self.next_append_label(
                        tenant_id, repo_id, branch, workspace, parent_id, revision,
                    )?;
                    get_last_time = t.elapsed().as_micros();

                    (new_label, true)
                }
            } else {
                // FAST PATH: New node - skip existence check, just calculate new label
                let t = std::time::Instant::now();
                let new_label = self.next_append_label(
                    tenant_id, repo_id, branch, workspace, parent_id, revision,
                )?;
                get_last_time = t.elapsed().as_micros();

                (new_label, true)
            };

            let label_calc_time = substep_start.elapsed().as_micros();

            // ========== SUBSTEP 4: Batch preparation ==========
            let substep_start = std::time::Instant::now();

            // Write to ordered children index
            // Store child name as value for efficient name-based lookups
            let ordered_key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_id,
                &order_label,
                revision,
                &node.id,
            );
            batch.put_cf(cf_ordered, ordered_key, node.name.as_bytes());

            // OPTIMIZATION: Update metadata cache for new children
            // This makes get_last_order_label() O(1) instead of O(n)
            if is_new_child {
                let metadata_key =
                    keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_id);
                batch.put_cf(cf_ordered, metadata_key, order_label.as_bytes());
            }

            // Keep the node record in step with the index entry (both on append
            // and when preserving an existing label, since the stored value may
            // have been stale).
            node.order_key = order_label;

            let batch_prep_time = substep_start.elapsed().as_micros();

            let total_order_time = order_start.elapsed().as_micros();

            // Log detailed order label timing breakdown
            tracing::debug!(
                "ORDER_TIMING node={} total={}us [parent={}us, exist={}us, calc={}us (get_existing={}us, get_last={}us), batch={}us]",
                node.name,
                total_order_time,
                parent_lookup_time,
                existence_check_time,
                label_calc_time,
                get_existing_time,
                get_last_time,
                batch_prep_time
            );

            // Also output to stderr for test visibility
            if std::env::var("RAISIN_PROFILE").is_ok() {
                eprintln!(
                    "ORDER_TIMING node={} total={}us [parent={}us, exist={}us, calc={}us (get_existing={}us, get_last={}us), batch={}us]",
                    node.name,
                    total_order_time,
                    parent_lookup_time,
                    existence_check_time,
                    label_calc_time,
                    get_existing_time,
                    get_last_time,
                    batch_prep_time
                );
            }
        }

        Ok(())
    }
}
