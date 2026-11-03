//! Single-Hop Pattern Matching
//!
//! Matches patterns of the form: (a)-[r]->(b)
//!
//! Uses RaisinDB's optimized relation indexes:
//! - Forward index: source -> targets (for outgoing traversal)
//! - Reverse index: target -> sources (for incoming traversal)
//! - Global index: all relations by type (for initial scans)

use std::sync::Arc;

use raisin_sql::ast::{Direction, NodePattern, RelationshipPattern};
use raisin_storage::{RelationRepository, Storage};

use super::matches_label;
use super::pushdown::{pinned_end, resolve_seek, side_for};
use super::seek::SeekHints;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::graph_access::{
    node_relations, RelationAccess, ScannedRelation, StorageAccess,
};
use crate::physical_plan::pgq::context::PgqContext;
use crate::physical_plan::pgq::types::{NodeInfo, RelationInfo, VariableBinding};

type Result<T> = std::result::Result<T, ExecutionError>;

/// Whether a relation's type satisfies the pattern's type list.
///
/// An empty list accepts every type — `-[r]->` is deliberately untyped. A list
/// of several is an ALTERNATION (`-[:road|walk]->`), and every named type must
/// be kept; keeping only the first is a silent wrong-results bug rather than a
/// narrower answer.
fn matches_relation_type(types: &[String], relation_type: &str) -> bool {
    types.is_empty() || types.iter().any(|t| t == relation_type)
}

/// Match a single-hop pattern: (source)-[rel]->(target)
///
/// Reads the candidate relations, then filters by:
/// 1. Relationship type (if specified)
/// 2. Source node label (if specified)
/// 3. Target node label (if specified)
///
/// The candidates come from the global relation index, O(relations of the
/// type), UNLESS `hints` pins one endpoint to a known node, in which case they
/// come from that node's own relation index, O(its degree). The `WHERE` clause
/// is still evaluated by the caller against every binding returned, so the
/// narrowing can only drop relations that could never have passed it.
///
/// # Arguments
///
/// * `source_pattern` - Pattern for the source node
/// * `rel_pattern` - Pattern for the relationship
/// * `target_pattern` - Pattern for the target node
/// * `hints` - Endpoints the `WHERE` clause pins to known nodes
/// * `storage` - Storage backend
/// * `context` - PGQ execution context
///
/// # Returns
///
/// Vector of variable bindings, one for each matched relationship.
pub async fn match_single_hop<S: Storage>(
    source_pattern: &NodePattern,
    rel_pattern: &RelationshipPattern,
    target_pattern: &NodePattern,
    hints: &SeekHints,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<VariableBinding>> {
    let access = StorageAccess::new(
        storage,
        &context.tenant_id,
        &context.repo_id,
        &context.branch,
        context.revision.as_ref(),
    )
    .with_workspace_hint(&context.workspace_id);
    match_single_hop_with(&access, source_pattern, rel_pattern, target_pattern, hints).await
}

/// The candidate relations for one hop.
async fn candidate_relations<A: RelationAccess>(
    access: &A,
    source_pattern: &NodePattern,
    rel_pattern: &RelationshipPattern,
    target_pattern: &NodePattern,
    hints: &SeekHints,
) -> Result<Vec<ScannedRelation>> {
    if let Some((end, seek)) = pinned_end(hints, source_pattern, target_pattern) {
        if let Some(nodes) = resolve_seek(access, seek).await? {
            let types = rel_pattern.types.clone();
            let accept = move |t: &str| matches_relation_type(&types, t);
            let relations = node_relations(
                access,
                &nodes,
                side_for(end, rel_pattern.direction),
                &accept,
            )
            .await?;
            tracing::debug!(
                "PGQ: single hop seeked {} node(s) through the per-node index: {} relations",
                nodes.len(),
                relations.len()
            );
            return Ok(relations);
        }
    }

    // Only a single-typed pattern can push its filter into storage:
    // `scan_relations_global` takes one `Option<&str>`. A multi-type alternation
    // such as `-[:road|walk]->` must therefore scan UNFILTERED and be filtered
    // in memory by the caller. Taking `types.first()` - as this once did -
    // silently dropped every `walk` edge and returned a strict subset of the
    // truth with no error anywhere, on a documented feature. Same defect, same
    // fix as `variable_length::scan_relations`.
    let relation_type_filter = match rel_pattern.types.as_slice() {
        [single] => Some(single.as_str()),
        _ => None,
    };
    let relations = access.global_scan(relation_type_filter).await?;
    tracing::debug!("PGQ: Found {} relations in global index", relations.len());
    Ok(relations)
}

/// [`match_single_hop`] over any [`RelationAccess`].
pub(crate) async fn match_single_hop_with<A: RelationAccess>(
    access: &A,
    source_pattern: &NodePattern,
    rel_pattern: &RelationshipPattern,
    target_pattern: &NodePattern,
    hints: &SeekHints,
) -> Result<Vec<VariableBinding>> {
    tracing::debug!(
        "PGQ: Matching single-hop pattern {:?}-[{:?}]->{:?}",
        source_pattern.labels,
        rel_pattern.types,
        target_pattern.labels
    );

    let relations =
        candidate_relations(access, source_pattern, rel_pattern, target_pattern, hints).await?;

    // Filter and create bindings
    let mut bindings = Vec::with_capacity(relations.len());

    for (src_ws, src_id, tgt_ws, tgt_id, full_rel) in relations {
        // The other half of the alternation fix: when the scan could not be
        // narrowed, keep only the types the pattern actually named.
        if !matches_relation_type(&rel_pattern.types, &full_rel.relation_type) {
            continue;
        }

        // Apply direction filter
        let (source_ws, source_id, source_type, target_ws, target_id, target_type) =
            match rel_pattern.direction {
                Direction::Right => {
                    // (a)-[r]->(b): source -> target
                    (
                        src_ws,
                        src_id,
                        &full_rel.source_node_type,
                        tgt_ws,
                        tgt_id,
                        &full_rel.target_node_type,
                    )
                }
                Direction::Left => {
                    // (a)<-[r]-(b): target <- source, so swap
                    (
                        tgt_ws,
                        tgt_id,
                        &full_rel.target_node_type,
                        src_ws,
                        src_id,
                        &full_rel.source_node_type,
                    )
                }
                Direction::Any => {
                    // (a)-[r]-(b): either direction matches
                    // For bidirectional, we include both directions
                    // First check forward direction
                    if matches_label(&source_pattern.labels, &full_rel.source_node_type)
                        && matches_label(&target_pattern.labels, &full_rel.target_node_type)
                    {
                        (
                            src_ws.clone(),
                            src_id.clone(),
                            &full_rel.source_node_type,
                            tgt_ws.clone(),
                            tgt_id.clone(),
                            &full_rel.target_node_type,
                        )
                    } else {
                        // Try reverse direction
                        (
                            tgt_ws,
                            tgt_id,
                            &full_rel.target_node_type,
                            src_ws,
                            src_id,
                            &full_rel.source_node_type,
                        )
                    }
                }
            };

        // Apply label filters
        if !matches_label(&source_pattern.labels, source_type) {
            continue;
        }
        if !matches_label(&target_pattern.labels, target_type) {
            continue;
        }

        // Create binding
        let mut binding = VariableBinding::new();

        // Bind source node if variable specified
        if let Some(var) = &source_pattern.variable {
            binding.bind_node(
                var.clone(),
                NodeInfo::new(source_id.clone(), source_ws.clone(), source_type.clone()),
            );
        }

        // Bind target node if variable specified
        if let Some(var) = &target_pattern.variable {
            binding.bind_node(
                var.clone(),
                NodeInfo::new(target_id.clone(), target_ws.clone(), target_type.clone()),
            );
        }

        // Bind relationship if variable specified
        if let Some(var) = &rel_pattern.variable {
            binding.bind_relation(
                var.clone(),
                RelationInfo::new(
                    full_rel.relation_type.clone(),
                    full_rel.weight,
                    source_pattern.variable.clone().unwrap_or_default(),
                    target_pattern.variable.clone().unwrap_or_default(),
                ),
            );
        }

        bindings.push(binding);
    }

    tracing::debug!("PGQ: Created {} bindings after filtering", bindings.len());

    Ok(bindings)
}

/// Match single-hop starting from a known source node
///
/// Uses the forward relation index for efficient traversal.
pub async fn match_from_source<S: Storage>(
    source_node: &NodeInfo,
    rel_pattern: &RelationshipPattern,
    target_pattern: &NodePattern,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<VariableBinding>> {
    // Get outgoing relations from source
    let relations = storage
        .relations()
        .get_outgoing_relations(
            raisin_storage::StorageScope::new(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &source_node.workspace,
            ),
            &source_node.id,
            context.revision.as_ref(),
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    let mut bindings = Vec::new();

    for rel_ref in relations {
        // Filter by relationship type
        if !rel_pattern.types.is_empty()
            && !rel_pattern
                .types
                .iter()
                .any(|t| t == &rel_ref.relation_type)
        {
            continue;
        }

        // Filter by target label
        if !matches_label(&target_pattern.labels, &rel_ref.target_node_type) {
            continue;
        }

        let mut binding = VariableBinding::new();

        // Bind target node
        if let Some(var) = &target_pattern.variable {
            binding.bind_node(
                var.clone(),
                NodeInfo::new(
                    rel_ref.target.clone(),
                    rel_ref.workspace.clone(),
                    rel_ref.target_node_type.clone(),
                ),
            );
        }

        // Bind relationship
        if let Some(var) = &rel_pattern.variable {
            binding.bind_relation(
                var.clone(),
                RelationInfo::new(
                    rel_ref.relation_type.clone(),
                    rel_ref.weight,
                    String::new(), // Source var not needed here
                    target_pattern.variable.clone().unwrap_or_default(),
                ),
            );
        }

        bindings.push(binding);
    }

    Ok(bindings)
}

/// Match single-hop ending at a known target node
///
/// Uses the reverse relation index for efficient traversal.
pub async fn match_to_target<S: Storage>(
    source_pattern: &NodePattern,
    rel_pattern: &RelationshipPattern,
    target_node: &NodeInfo,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<Vec<VariableBinding>> {
    // Get incoming relations to target
    let relations = storage
        .relations()
        .get_incoming_relations(
            raisin_storage::StorageScope::new(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &target_node.workspace,
            ),
            &target_node.id,
            context.revision.as_ref(),
        )
        .await
        .map_err(|e| ExecutionError::Backend(e.to_string()))?;

    let mut bindings = Vec::new();

    for (source_ws, source_id, rel_ref) in relations {
        // Filter by relationship type
        if !rel_pattern.types.is_empty()
            && !rel_pattern
                .types
                .iter()
                .any(|t| t == &rel_ref.relation_type)
        {
            continue;
        }

        // Filter by source label
        // Note: target_node_type in reverse index is actually the source node type
        if !matches_label(&source_pattern.labels, &rel_ref.target_node_type) {
            continue;
        }

        let mut binding = VariableBinding::new();

        // Bind source node
        if let Some(var) = &source_pattern.variable {
            binding.bind_node(
                var.clone(),
                NodeInfo::new(source_id, source_ws, rel_ref.target_node_type.clone()),
            );
        }

        // Bind relationship
        if let Some(var) = &rel_pattern.variable {
            binding.bind_relation(
                var.clone(),
                RelationInfo::new(
                    rel_ref.relation_type.clone(),
                    rel_ref.weight,
                    source_pattern.variable.clone().unwrap_or_default(),
                    String::new(), // Target var not needed here
                ),
            );
        }

        bindings.push(binding);
    }

    Ok(bindings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relation_info() {
        let rel = RelationInfo::new("FOLLOWS".into(), Some(0.9), "a".into(), "b".into());
        assert_eq!(rel.relation_type, "FOLLOWS");
        assert_eq!(rel.weight, Some(0.9));
    }
}
