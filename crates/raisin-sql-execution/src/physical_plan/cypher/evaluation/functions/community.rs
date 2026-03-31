//! Community detection functions for Cypher
//!
//! Provides functions to identify connected components and communities in graphs.

use std::collections::HashMap;

use raisin_cypher_parser::Expr;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{RelationRepository, Storage};

use super::super::expr::evaluate_expr_async_impl;
use super::traits::FunctionContext;
use crate::physical_plan::cypher::algorithms::GraphAdjacency;
use crate::physical_plan::cypher::types::VariableBinding;

/// Helper: Extract node ID and workspace from expression
async fn extract_node_id_workspace<S: Storage>(
    expr: &Expr,
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<(String, String), Error> {
    let node_value = evaluate_expr_async_impl(expr, binding, context).await?;

    match node_value {
        PropertyValue::Object(ref map) => {
            let id = map
                .get("id")
                .and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| Error::Validation("Node must have an 'id' field".to_string()))?;

            let workspace = map
                .get("workspace")
                .and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    Error::Validation("Node must have a 'workspace' field".to_string())
                })?;

            Ok((id, workspace))
        }
        _ => Err(Error::Validation(
            "Community functions require node objects as arguments".to_string(),
        )),
    }
}

/// Helper: Build adjacency graph from current query context
async fn build_adjacency_graph<S: Storage>(
    context: &FunctionContext<'_, S>,
) -> Result<GraphAdjacency, Error> {
    tracing::debug!("   Building adjacency graph for community detection...");

    let all_relationships = context
        .storage
        .relations()
        .scan_relations_global(
            raisin_storage::BranchScope::new(context.tenant_id, context.repo_id, context.branch),
            None,
            context.revision,
        )
        .await
        .map_err(|e| Error::Backend(e.to_string()))?;

    tracing::debug!("   Scanned {} relationships", all_relationships.len());

    let mut adjacency: GraphAdjacency = HashMap::new();

    for (src_workspace, src_id, tgt_workspace, tgt_id, rel_ref) in all_relationships {
        let key = (src_workspace, src_id);
        let value = (tgt_workspace, tgt_id, rel_ref.relation_type);
        adjacency.entry(key).or_default().push(value);
    }

    tracing::debug!("   Built adjacency graph with {} nodes", adjacency.len());

    Ok(adjacency)
}

/// componentId(node) - Get connected component ID for a node
///
/// Returns the component ID (integer). Nodes in the same weakly connected component
/// share the same ID.
///
/// # Arguments
///
/// * `args` - Must contain exactly 1 expression: the node variable
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing component ID, or -1 if node not found
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Wrong number of arguments (not 1)
/// - Argument is not a node object
///
/// Returns Error::Backend if:
/// - Storage operation fails
///
/// # Example
///
/// ```cypher
/// MATCH (n)
/// RETURN n.id, componentId(n) AS component
/// ```
pub async fn evaluate_component_id<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() != 1 {
        return Err(Error::Validation(
            "componentId() expects 1 argument: componentId(node)".to_string(),
        ));
    }

    tracing::debug!(" → Evaluating componentId()");

    // Extract node (workspace, id)
    let (node_id, node_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;

    tracing::debug!("   - node: {} (workspace: {})", node_id, node_workspace);

    // Build adjacency graph
    tracing::debug!("   - Building adjacency graph for component detection...");
    let adjacency = build_adjacency_graph(context).await?;

    // Get component ID for this node
    let node_key = (node_workspace, node_id.clone());
    let component_id =
        crate::physical_plan::cypher::algorithms::node_component_id(&adjacency, &node_key);

    match component_id {
        Some(id) => {
            tracing::debug!("   ✓ componentId({}) = {}", node_id, id);
            Ok(PropertyValue::Integer(id as i64))
        }
        None => {
            tracing::debug!("   ✓ componentId({}) = -1 (node not found)", node_id);
            Ok(PropertyValue::Integer(-1)) // Node not in graph
        }
    }
}

/// componentCount() - Get total number of connected components
///
/// Returns the count of weakly connected components in the graph.
///
/// # Arguments
///
/// * `args` - Must be empty (no arguments)
/// * `binding` - Current variable binding (unused)
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing count of components
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Any arguments are provided
///
/// Returns Error::Backend if:
/// - Storage operation fails
///
/// # Example
///
/// ```cypher
/// RETURN componentCount() AS numComponents
/// ```
pub async fn evaluate_component_count<S: Storage>(
    args: &[Expr],
    _binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if !args.is_empty() {
        return Err(Error::Validation(
            "componentCount() expects no arguments".to_string(),
        ));
    }

    tracing::debug!(" → Evaluating componentCount()");

    // Build adjacency graph
    tracing::debug!("   - Building adjacency graph for component counting...");
    let adjacency = build_adjacency_graph(context).await?;

    // Count components
    let count = crate::physical_plan::cypher::algorithms::component_count(&adjacency);

    tracing::debug!("   ✓ componentCount() = {}", count);

    Ok(PropertyValue::Integer(count as i64))
}

/// communityId(node) - Get community ID for a node using Label Propagation
///
/// Returns the community ID (integer). Nodes in the same community share the same ID.
/// Uses the Label Propagation Algorithm to detect communities.
///
/// # Arguments
///
/// * `args` - Must contain exactly 1 expression: the node variable
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing community ID, or -1 if node not found
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Wrong number of arguments (not 1)
/// - Argument is not a node object
///
/// Returns Error::Backend if:
/// - Storage operation fails
///
/// # Example
///
/// ```cypher
/// MATCH (n)
/// RETURN n.id, communityId(n) AS community
/// ```
pub async fn evaluate_community_id<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() != 1 {
        return Err(Error::Validation(
            "communityId() expects 1 argument: communityId(node)".to_string(),
        ));
    }

    tracing::debug!(" → Evaluating communityId()");

    // Extract node (workspace, id)
    let (node_id, node_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;

    tracing::debug!("   - node: {} (workspace: {})", node_id, node_workspace);

    // Build adjacency graph
    tracing::debug!("   - Building adjacency graph for community detection...");
    let adjacency = build_adjacency_graph(context).await?;

    // Get community ID for this node
    let node_key = (node_workspace, node_id.clone());
    let community_id =
        crate::physical_plan::cypher::algorithms::node_community_id(&adjacency, &node_key);

    match community_id {
        Some(id) => {
            tracing::debug!("   ✓ communityId({}) = {}", node_id, id);
            Ok(PropertyValue::Integer(id as i64))
        }
        None => {
            tracing::debug!("   ✓ communityId({}) = -1 (node not found)", node_id);
            Ok(PropertyValue::Integer(-1)) // Node not in graph
        }
    }
}

/// communityCount() - Get total number of communities detected
///
/// Returns the count of communities using Label Propagation Algorithm.
///
/// # Arguments
///
/// * `args` - Must be empty (no arguments)
/// * `binding` - Current variable binding (unused)
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing count of communities
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Any arguments are provided
///
/// Returns Error::Backend if:
/// - Storage operation fails
///
/// # Example
///
/// ```cypher
/// RETURN communityCount() AS numCommunities
/// ```
pub async fn evaluate_community_count<S: Storage>(
    args: &[Expr],
    _binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if !args.is_empty() {
        return Err(Error::Validation(
            "communityCount() expects no arguments".to_string(),
        ));
    }

    tracing::debug!(" → Evaluating communityCount()");

    // Build adjacency graph
    tracing::debug!("   - Building adjacency graph for community counting...");
    let adjacency = build_adjacency_graph(context).await?;

    // Count communities
    let count = crate::physical_plan::cypher::algorithms::community_count(&adjacency);

    tracing::debug!("   ✓ communityCount() = {}", count);

    Ok(PropertyValue::Integer(count as i64))
}

/// louvain(node) - Get community ID for a node using Louvain Algorithm
///
/// Returns the community ID (integer). Nodes in the same community share the same ID.
/// Uses the Louvain Algorithm to detect communities by optimizing modularity.
///
/// # Arguments
///
/// * `args` - Must contain exactly 1 expression: the node variable
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing community ID, or -1 if node not found
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Wrong number of arguments (not 1)
/// - Argument is not a node object
///
/// Returns Error::Backend if:
/// - Storage operation fails
///
/// # Example
///
/// ```cypher
/// MATCH (n)
/// RETURN n.id, louvain(n) AS community
/// ```
pub async fn evaluate_louvain<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() != 1 {
        return Err(Error::Validation(
            "louvain() expects 1 argument: louvain(node)".to_string(),
        ));
    }

    tracing::debug!(" → Evaluating louvain()");

    // Extract node (workspace, id)
    let (node_id, node_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;

    tracing::debug!("   - node: {} (workspace: {})", node_id, node_workspace);

    // Build adjacency graph
    tracing::debug!("   - Building adjacency graph for Louvain community detection...");
    let adjacency = build_adjacency_graph(context).await?;

    // Get community ID for this node
    let node_key = (node_workspace, node_id.clone());
    let community_id =
        crate::physical_plan::cypher::algorithms::node_louvain_community_id(&adjacency, &node_key);

    match community_id {
        Some(id) => {
            tracing::debug!("   ✓ louvain({}) = {}", node_id, id);
            Ok(PropertyValue::Integer(id as i64))
        }
        None => {
            tracing::debug!("   ✓ louvain({}) = -1 (node not found)", node_id);
            Ok(PropertyValue::Integer(-1)) // Node not in graph
        }
    }
}

/// triangleCount(node) - Get number of triangles a node participates in
///
/// Returns the number of triangles (cycles of length 3) that include the given node.
///
/// # Arguments
///
/// * `args` - Must contain exactly 1 expression: the node variable
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing triangle count
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Wrong number of arguments (not 1)
/// - Argument is not a node object
///
/// Returns Error::Backend if:
/// - Storage operation fails
///
/// # Example
///
/// ```cypher
/// MATCH (n)
/// RETURN n.id, triangleCount(n) AS triangles
/// ```
pub async fn evaluate_triangle_count<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() != 1 {
        return Err(Error::Validation(
            "triangleCount() expects 1 argument: triangleCount(node)".to_string(),
        ));
    }

    tracing::debug!(" → Evaluating triangleCount()");

    // Extract node (workspace, id)
    let (node_id, node_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;

    tracing::debug!("   - node: {} (workspace: {})", node_id, node_workspace);

    // Build adjacency graph
    tracing::debug!("   - Building adjacency graph for triangle counting...");
    let adjacency = build_adjacency_graph(context).await?;

    // Get triangle count for this node
    let node_key = (node_workspace, node_id.clone());
    let count =
        crate::physical_plan::cypher::algorithms::node_triangle_count(&adjacency, &node_key);

    tracing::debug!("   ✓ triangleCount({}) = {}", node_id, count);
    Ok(PropertyValue::Integer(count as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_community_functions_signatures() {
        // This test just ensures the function signatures are correct
        // Full testing requires a mock storage implementation
    }
}
