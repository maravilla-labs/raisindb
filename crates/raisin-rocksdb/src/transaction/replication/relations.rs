//! Relation operation capture for replication.

use super::super::RocksDBTransaction;
use raisin_error::Result;
use raisin_hlc::HLC;
use std::collections::HashMap;

impl RocksDBTransaction {
    /// Capture relation operations (AddRelation/RemoveRelation) from tracked changes
    pub(in super::super) async fn capture_relation_operations(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        tracked_changes: &HashMap<String, crate::replication::NodeChanges>,
        actor: String,
        is_system: bool,
        revision: HLC,
    ) -> Result<()> {
        tracing::info!(
            "🔍 capture_relation_operations called with {} node changes",
            tracked_changes.len()
        );

        let mut relation_count = 0;

        // Iterate through all node changes and extract relation changes
        for (node_id, node_changes) in tracked_changes.iter() {
            if !node_changes.relation_changes.is_empty() {
                tracing::info!(
                    "🔍 Node {} has {} relation changes",
                    node_id,
                    node_changes.relation_changes.len()
                );
                eprintln!(
                    "🔍 CAPTURE_RELATION_OPS: Node {} has {} relation changes",
                    node_id,
                    node_changes.relation_changes.len()
                );
            }

            for rel_change in &node_changes.relation_changes {
                relation_count += 1;

                let op_type = if rel_change.is_addition {
                    let relation = rel_change.relation.clone().ok_or_else(|| {
                        raisin_error::Error::storage(
                            "Relation addition missing relation payload".to_string(),
                        )
                    })?;

                    tracing::info!(
                        "📤 Capturing AddRelation: {} --[{}]--> {}",
                        rel_change.source_id,
                        rel_change.relation_type,
                        rel_change.target_id
                    );
                    eprintln!(
                        "📤 CAPTURE: AddRelation: {} --[{}]--> {}",
                        rel_change.source_id, rel_change.relation_type, rel_change.target_id
                    );

                    raisin_replication::OpType::AddRelation {
                        source_id: rel_change.source_id.clone(),
                        source_workspace: rel_change.source_workspace.clone(),
                        relation_type: rel_change.relation_type.clone(),
                        target_id: rel_change.target_id.clone(),
                        target_workspace: rel_change.target_workspace.clone(),
                        relation,
                    }
                } else {
                    tracing::info!(
                        "📤 Capturing RemoveRelation: {} --[{}]--> {}",
                        rel_change.source_id,
                        rel_change.relation_type,
                        rel_change.target_id
                    );
                    eprintln!(
                        "📤 CAPTURE: RemoveRelation: {} --[{}]--> {}",
                        rel_change.source_id, rel_change.relation_type, rel_change.target_id
                    );

                    raisin_replication::OpType::RemoveRelation {
                        source_id: rel_change.source_id.clone(),
                        source_workspace: rel_change.source_workspace.clone(),
                        relation_type: rel_change.relation_type.clone(),
                        target_id: rel_change.target_id.clone(),
                        target_workspace: rel_change.target_workspace.clone(),
                    }
                };

                // Capture the relation operation
                self.capture_operation_internal(
                    tenant_id.clone(),
                    repo_id.clone(),
                    branch.clone(),
                    op_type,
                    actor.clone(),
                    None, // No message for individual relation ops
                    is_system,
                    Some(revision),
                )
                .await;
            }
        }

        if relation_count > 0 {
            tracing::info!(
                "✅ Captured {} relation operations for replication",
                relation_count
            );
        } else {
            tracing::info!("⚠️ No relation operations to capture");
        }

        Ok(())
    }
}
