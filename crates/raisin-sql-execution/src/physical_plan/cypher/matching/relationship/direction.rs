//! Relationship direction and type filtering
//!
//! Fetches outgoing, incoming, or bidirectional relationships from storage
//! and filters them by relationship type.

use std::sync::Arc;

use raisin_cypher_parser::{Direction, RelPattern};
use raisin_models::nodes::RelationRef;
use raisin_storage::{RelationRepository, Storage};

use crate::physical_plan::cypher::types::CypherContext;
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Get relationships based on pattern direction
///
/// Fetches outgoing, incoming, or both relationships depending on the pattern direction.
pub async fn get_relations_by_direction<S: Storage>(
    node_id: &str,
    workspace: &str,
    rel_pattern: &RelPattern,
    storage: &Arc<S>,
    context: &CypherContext,
) -> Result<Vec<RelationRef>> {
    match rel_pattern.direction {
        Direction::Right => storage
            .relations()
            .get_outgoing_relations(
                raisin_storage::StorageScope::new(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    workspace,
                ),
                node_id,
                context.revision.as_ref(),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string())),
        Direction::Left => storage
            .relations()
            .get_incoming_relations(
                raisin_storage::StorageScope::new(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    workspace,
                ),
                node_id,
                context.revision.as_ref(),
            )
            .await
            .map_err(|e| ExecutionError::Backend(e.to_string()))
            .map(|rels| {
                rels.into_iter()
                    .map(|(source_workspace, source_id, rel)| RelationRef {
                        target: source_id,
                        workspace: source_workspace,
                        target_node_type: rel.target_node_type,
                        relation_type: rel.relation_type,
                        weight: rel.weight,
                    })
                    .collect()
            }),
        Direction::Both | Direction::None => {
            let mut outgoing = storage
                .relations()
                .get_outgoing_relations(
                    raisin_storage::StorageScope::new(
                        &context.tenant_id,
                        &context.repo_id,
                        &context.branch,
                        workspace,
                    ),
                    node_id,
                    context.revision.as_ref(),
                )
                .await
                .map_err(|e| ExecutionError::Backend(e.to_string()))?;

            let incoming = storage
                .relations()
                .get_incoming_relations(
                    raisin_storage::StorageScope::new(
                        &context.tenant_id,
                        &context.repo_id,
                        &context.branch,
                        workspace,
                    ),
                    node_id,
                    context.revision.as_ref(),
                )
                .await
                .map_err(|e| ExecutionError::Backend(e.to_string()))?
                .into_iter()
                .map(|(source_workspace, source_id, rel)| RelationRef {
                    target: source_id,
                    workspace: source_workspace,
                    target_node_type: rel.target_node_type,
                    relation_type: rel.relation_type,
                    weight: rel.weight,
                });

            outgoing.extend(incoming);
            Ok(outgoing)
        }
    }
}

/// Filter relationships by type
///
/// Returns only relationships whose type matches one of the specified types.
/// If no types are specified, returns all relationships.
pub fn filter_relations_by_type(relations: Vec<RelationRef>, types: &[String]) -> Vec<RelationRef> {
    if types.is_empty() {
        return relations;
    }

    if types.len() > 3 {
        let type_set: std::collections::HashSet<_> = types.iter().collect();
        relations
            .into_iter()
            .filter(|rel| type_set.contains(&rel.relation_type))
            .collect()
    } else {
        relations
            .into_iter()
            .filter(|rel| types.contains(&rel.relation_type))
            .collect()
    }
}
