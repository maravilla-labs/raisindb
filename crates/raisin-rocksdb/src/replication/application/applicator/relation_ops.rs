//! Relation operations: AddRelation, RemoveRelation

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::RelationRef;
use raisin_replication::Operation;

use super::OperationApplicator;

impl OperationApplicator {
    /// Apply an AddRelation operation
    ///
    /// Writes to both forward (source->target) and reverse (target->source) relation indexes.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::replication::application) async fn apply_add_relation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        source_id: &str,
        source_workspace: &str,
        relation_type: &str,
        target_id: &str,
        target_workspace: &str,
        relation: RelationRef,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying AddRelation: {} --[{}]--> {}",
            source_id,
            relation_type,
            target_id
        );
        eprintln!(
            "📥 APPLY_ADD_RELATION: {} --[{}]--> {} (tenant={}, repo={}, branch={})",
            source_id, relation_type, target_id, tenant_id, repo_id, branch
        );

        let revision = Self::op_revision(op)?;

        let forward_relation_bytes = rmp_serde::to_vec(&relation)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let reverse_relation = RelationRef::new(
            source_id.to_string(),
            source_workspace.to_string(),
            relation.target_node_type.clone(),
            relation.relation_type.clone(),
            relation.weight,
        );
        let reverse_relation_bytes = rmp_serde::to_vec(&reverse_relation)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        // Write to forward index
        let forward_key = keys::relation_forward_key_versioned(
            tenant_id,
            repo_id,
            branch,
            source_workspace,
            source_id,
            relation_type,
            &revision,
            target_id,
        );

        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;
        self.db
            .put_cf(cf_relation, forward_key, &forward_relation_bytes)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Write to reverse index
        let reverse_key = keys::relation_reverse_key_versioned(
            tenant_id,
            repo_id,
            branch,
            target_workspace,
            target_id,
            relation_type,
            &revision,
            source_id,
        );

        self.db
            .put_cf(cf_relation, reverse_key, &reverse_relation_bytes)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::info!(
            "✅ Relation added: {} --[{}]--> {}",
            source_id,
            relation_type,
            target_id
        );

        super::super::relation_operations::emit_relation_events(
            self,
            tenant_id,
            repo_id,
            branch,
            source_workspace,
            source_id,
            target_workspace,
            target_id,
            relation_type,
            &revision,
            raisin_events::NodeEventKind::RelationAdded {
                relation_type: relation_type.to_string(),
                target_node_id: target_id.to_string(),
            },
        );

        Ok(())
    }

    /// Apply a RemoveRelation operation
    ///
    /// Removes from both forward and reverse relation indexes using tombstones.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::replication::application) async fn apply_remove_relation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        source_id: &str,
        source_workspace: &str,
        relation_type: &str,
        target_id: &str,
        target_workspace: &str,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying RemoveRelation: {} --[{}]--> {}",
            source_id,
            relation_type,
            target_id
        );
        eprintln!(
            "📥 APPLY_REMOVE_RELATION: {} --[{}]--> {} (tenant={}, repo={}, branch={})",
            source_id, relation_type, target_id, tenant_id, repo_id, branch
        );

        let revision = Self::op_revision(op)?;

        let forward_key = keys::relation_forward_key_versioned(
            tenant_id,
            repo_id,
            branch,
            source_workspace,
            source_id,
            relation_type,
            &revision,
            target_id,
        );

        let reverse_key = keys::relation_reverse_key_versioned(
            tenant_id,
            repo_id,
            branch,
            target_workspace,
            target_id,
            relation_type,
            &revision,
            source_id,
        );

        let global_key = keys::relation_global_key_versioned(
            tenant_id,
            repo_id,
            branch,
            relation_type,
            &revision,
            source_workspace,
            source_id,
            target_workspace,
            target_id,
        );

        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;

        // Write tombstones to all three indexes
        self.db
            .put_cf(cf_relation, forward_key, b"T")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        self.db
            .put_cf(cf_relation, reverse_key, b"T")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        self.db
            .put_cf(cf_relation, global_key, b"T")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        tracing::info!(
            "✅ Relation removed: {} --[{}]--> {}",
            source_id,
            relation_type,
            target_id
        );
        eprintln!(
            "📥 ✅ APPLY_REMOVE_RELATION COMPLETE: {} --[{}]--> {}",
            source_id, relation_type, target_id
        );

        super::super::relation_operations::emit_relation_events(
            self,
            tenant_id,
            repo_id,
            branch,
            source_workspace,
            source_id,
            target_workspace,
            target_id,
            relation_type,
            &revision,
            raisin_events::NodeEventKind::RelationRemoved {
                relation_type: relation_type.to_string(),
                target_node_id: target_id.to_string(),
            },
        );

        Ok(())
    }
}
