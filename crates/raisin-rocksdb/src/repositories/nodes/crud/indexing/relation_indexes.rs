//! Relation index operations (forward and reverse)

use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Add relation indexes (forward and reverse) for all node relations
    pub(in crate::repositories) fn add_relation_indexes(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;

        for relation in &node.relations {
            // Serialize the relation to MessagePack
            let relation_bytes = rmp_serde::to_vec(&relation).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to serialize relation: {}", e))
            })?;

            // Forward relation index: source -> target (for outgoing queries)
            let fwd_key = keys::relation_forward_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node.id,
                &relation.relation_type,
                revision,
                &relation.target,
            );
            batch.put_cf(cf_relation, fwd_key, &relation_bytes);

            // Reverse relation index: target -> source (for incoming queries)
            let rev_key = keys::relation_reverse_key_versioned(
                tenant_id,
                repo_id,
                branch,
                &relation.workspace,
                &relation.target,
                &relation.relation_type,
                revision,
                &node.id,
            );
            batch.put_cf(cf_relation, rev_key, &relation_bytes);
        }

        Ok(())
    }
}
