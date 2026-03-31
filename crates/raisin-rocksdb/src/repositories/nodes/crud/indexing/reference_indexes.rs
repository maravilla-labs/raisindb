//! Reference index operations (forward and reverse)

use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Add reference indexes (forward and reverse) for all PropertyValue::Reference entries
    pub(in crate::repositories) fn add_reference_indexes(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf_reference = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        let is_published = node.published_at.is_some();

        // Extract references from properties (looking for PropertyValue::Reference)
        for (prop_path, prop_value) in &node.properties {
            if let PropertyValue::Reference(ref_data) = prop_value {
                // Forward reference index (from this node to target)
                let fwd_key = keys::reference_forward_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node.id,
                    prop_path,
                    revision,
                    is_published,
                );
                batch.put_cf(cf_reference, fwd_key, ref_data.id.as_bytes());

                // Reverse reference index (from target to this node)
                let rev_key = keys::reference_reverse_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &ref_data.workspace,
                    &ref_data.path,
                    &node.id,
                    prop_path,
                    revision,
                    is_published,
                );
                batch.put_cf(cf_reference, rev_key, node.id.as_bytes());
            }
        }

        Ok(())
    }
}
