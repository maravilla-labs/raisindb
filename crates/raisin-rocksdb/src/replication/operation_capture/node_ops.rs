//! Convenience methods for relation operation capture
//!
//! Thin wrappers around `capture_operation` that construct the relation
//! `OpType` variants. Node writes have no per-field builders: they replicate
//! as one `ApplyRevision` snapshot (`repositories/nodes/replication_capture.rs`,
//! `transaction/replication/`); the pre-v2 granular node ops and their
//! builders are gone (plan "Phase 11d").

use raisin_error::Result;
use raisin_replication::{OpType, Operation};

use super::core::OperationCapture;

impl OperationCapture {
    /// Capture an AddRelation operation.
    /// Uses Last-Write-Wins (LWW) semantics based on HLC timestamps.
    pub async fn capture_add_relation(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        source_id: String,
        source_workspace: String,
        relation_type: String,
        target_id: String,
        target_workspace: String,
        properties: serde_json::Value,
        actor: String,
    ) -> Result<Operation> {
        let props: std::collections::HashMap<String, serde_json::Value> =
            serde_json::from_value(properties).unwrap_or_default();

        let target_node_type = props
            .get("target_node_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let weight = props
            .get("weight")
            .and_then(|v| v.as_f64())
            .map(|w| w as f32);

        let relation = raisin_models::nodes::RelationRef::new(
            target_id,
            target_workspace,
            target_node_type,
            relation_type.clone(),
            weight,
        );

        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::AddRelation {
                source_id,
                source_workspace,
                relation_type,
                target_id: relation.target.clone(),
                target_workspace: relation.workspace.clone(),
                relation,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a RemoveRelation operation.
    /// Uses Last-Write-Wins (LWW) semantics based on HLC timestamps.
    pub async fn capture_remove_relation(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        source_id: String,
        source_workspace: String,
        relation_type: String,
        target_id: String,
        target_workspace: String,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::RemoveRelation {
                source_id,
                source_workspace,
                relation_type,
                target_id,
                target_workspace,
            },
            actor,
            None,
            false,
        )
        .await
    }
}
