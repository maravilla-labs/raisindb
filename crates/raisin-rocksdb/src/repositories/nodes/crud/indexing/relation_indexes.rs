//! Relation index operations (forward and reverse)

use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::{FullRelation, Node, RelationRef};
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Add relation indexes (forward, reverse and global) for all node relations.
    ///
    /// Writes the same three entries as `RelationRepository::add_relation`
    /// and the transaction's `add_relation`: the reverse value describes the
    /// SOURCE (its id, workspace and type), because `get_incoming_relations`
    /// reads the source's workspace from it. Writing the forward value there
    /// made an incoming lookup across workspaces load the source node from
    /// the target's workspace, and without the global entry Cypher / PGQ and
    /// the NEIGHBORS fallback never saw these relations.
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
            let reverse = RelationRef::new(
                node.id.clone(),
                workspace.to_string(),
                node.node_type.clone(),
                relation.relation_type.clone(),
                relation.weight,
            );
            let reverse_bytes = rmp_serde::to_vec(&reverse).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to serialize relation: {}", e))
            })?;
            batch.put_cf(cf_relation, rev_key, &reverse_bytes);

            // Global index: every relation of the branch (Cypher, PGQ, export)
            let full = FullRelation::from_source_and_ref(
                node.id.clone(),
                workspace.to_string(),
                node.node_type.clone(),
                relation,
            );
            let full_bytes = rmp_serde::to_vec(&full).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to serialize relation: {}", e))
            })?;
            let global_key = keys::relation_global_key_versioned(
                tenant_id,
                repo_id,
                branch,
                &relation.relation_type,
                revision,
                workspace,
                &node.id,
                &relation.workspace,
                &relation.target,
            );
            batch.put_cf(cf_relation, global_key, &full_bytes);
        }

        Ok(())
    }
}
