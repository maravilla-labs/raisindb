//! Centrality analysis functions for Cypher
//!
//! Provides functions to measure node importance using various centrality algorithms.

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
            "Centrality functions require node objects as arguments".to_string(),
        )),
    }
}

/// Helper: Build adjacency graph from current query context
async fn build_adjacency_graph<S: Storage>(
    context: &FunctionContext<'_, S>,
) -> Result<GraphAdjacency, Error> {
    tracing::debug!("   Building adjacency graph for centrality calculation...");

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

/// pageRank(node, dampingFactor?, maxIterations?) - Calculate PageRank score
///
/// Returns the PageRank centrality score for the node (0.0 to 1.0).
/// PageRank measures importance based on incoming links.
///
/// # Arguments
///
/// * `args` - Must contain 1-3 expressions: [node, dampingFactor?, maxIterations?]
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing PageRank score (0.0 to 1.0)
///
/// # Errors
///
/// Returns Error::Validation if:
/// - Wrong number of arguments (not 1-3)
/// - Arguments have invalid types
/// - dampingFactor not between 0 and 1
/// - maxIterations less than 1
///
/// Returns Error::Backend if:
/// - Storage operation fails
///
/// # Example
///
/// ```cypher
/// MATCH (n)
/// RETURN n.id, pageRank(n, 0.85, 100) AS rank
/// ORDER BY rank DESC
/// ```
pub async fn evaluate_pagerank<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.is_empty() || args.len() > 3 {
        return Err(Error::Validation(
            "pageRank() requires 1-3 arguments (node, dampingFactor?, maxIterations?)".to_string(),
        ));
    }

    // Extract node
    let (node_id, node_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;

    // Extract optional damping factor (default 0.85)
    let damping_factor = if args.len() >= 2 {
        match evaluate_expr_async_impl(&args[1], binding, context).await? {
            PropertyValue::Integer(n) => {
                let f = n as f64;
                if f <= 0.0 || f >= 1.0 {
                    return Err(Error::Validation(
                        "pageRank() damping factor must be between 0 and 1".to_string(),
                    ));
                }
                f
            }
            PropertyValue::Float(n) => {
                if n <= 0.0 || n >= 1.0 {
                    return Err(Error::Validation(
                        "pageRank() damping factor must be between 0 and 1".to_string(),
                    ));
                }
                n
            }
            _ => {
                return Err(Error::Validation(
                    "pageRank() second argument (dampingFactor) must be a number".to_string(),
                ))
            }
        }
    } else {
        0.85 // Default damping factor
    };

    // Extract optional max iterations (default 100)
    let max_iterations = if args.len() == 3 {
        match evaluate_expr_async_impl(&args[2], binding, context).await? {
            PropertyValue::Integer(n) => {
                if n < 1 {
                    return Err(Error::Validation(
                        "pageRank() maxIterations must be >= 1".to_string(),
                    ));
                }
                n as usize
            }
            PropertyValue::Float(n) => {
                if n < 1.0 {
                    return Err(Error::Validation(
                        "pageRank() maxIterations must be >= 1".to_string(),
                    ));
                }
                n as usize
            }
            _ => {
                return Err(Error::Validation(
                    "pageRank() third argument (maxIterations) must be a number".to_string(),
                ))
            }
        }
    } else {
        100 // Default max iterations
    };

    tracing::debug!(
        "   pageRank({}:{}, damping={}, maxIter={})",
        node_workspace,
        node_id,
        damping_factor,
        max_iterations
    );

    // Build adjacency graph
    let adjacency = build_adjacency_graph(context).await?;

    // Calculate PageRank for all nodes
    let config = crate::physical_plan::cypher::algorithms::PageRankConfig {
        damping_factor,
        max_iterations,
        convergence_threshold: 0.0001,
    };

    let pagerank_scores = crate::physical_plan::cypher::algorithms::pagerank(&adjacency, &config);

    // Get score for requested node
    let node_key = (node_workspace, node_id.clone());
    let score = pagerank_scores.get(&node_key).copied().unwrap_or(0.0);

    tracing::debug!("   ✓ PageRank({}) = {:.6}", node_id, score);

    Ok(PropertyValue::Float(score))
}

/// closeness(node) - Calculate closeness centrality
///
/// Returns the closeness centrality score (0.0 to 1.0).
/// Closeness measures how close a node is to all other reachable nodes.
///
/// # Arguments
///
/// * `args` - Must contain exactly 1 expression: the node variable
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing closeness centrality (0.0 to 1.0)
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
/// RETURN n.id, closeness(n) AS closeness
/// ORDER BY closeness DESC
/// ```
pub async fn evaluate_closeness<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() != 1 {
        return Err(Error::Validation(
            "closeness() expects 1 argument: closeness(node)".to_string(),
        ));
    }

    tracing::debug!(" → Evaluating closeness()");

    // Extract node (workspace, id)
    let (node_id, node_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;

    tracing::debug!("   - node: {} (workspace: {})", node_id, node_workspace);

    // Build adjacency graph
    tracing::debug!("   - Building adjacency graph for closeness calculation...");
    let adjacency = build_adjacency_graph(context).await?;

    // Calculate closeness centrality
    let node_key = (node_workspace, node_id.clone());
    let score =
        crate::physical_plan::cypher::algorithms::closeness_centrality(&adjacency, &node_key);

    tracing::debug!("   ✓ closeness({}) = {:.6}", node_id, score);

    Ok(PropertyValue::Float(score))
}

/// betweenness(node) - Calculate betweenness centrality
///
/// Returns the betweenness centrality score (0.0 to 1.0).
/// Betweenness measures how often a node lies on shortest paths between other nodes.
///
/// # Arguments
///
/// * `args` - Must contain exactly 1 expression: the node variable
/// * `binding` - Current variable binding for evaluating expressions
/// * `context` - Function evaluation context with storage access
///
/// # Returns
///
/// Number representing betweenness centrality (0.0 to 1.0)
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
/// RETURN n.id, betweenness(n) AS betweenness
/// ORDER BY betweenness DESC
/// ```
pub async fn evaluate_betweenness<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() != 1 {
        return Err(Error::Validation(
            "betweenness() expects 1 argument: betweenness(node)".to_string(),
        ));
    }

    tracing::debug!(" → Evaluating betweenness()");

    // Extract node (workspace, id)
    let (node_id, node_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;

    tracing::debug!("   - node: {} (workspace: {})", node_id, node_workspace);

    // Build adjacency graph
    tracing::debug!("   - Building adjacency graph for betweenness calculation...");
    let adjacency = build_adjacency_graph(context).await?;

    // Calculate betweenness centrality
    let node_key = (node_workspace, node_id.clone());
    let score =
        crate::physical_plan::cypher::algorithms::betweenness_centrality(&adjacency, &node_key);

    tracing::debug!("   ✓ betweenness({}) = {:.6}", node_id, score);

    Ok(PropertyValue::Float(score))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_centrality_functions_signatures() {
        // This test just ensures the function signatures are correct
        // Full testing requires a mock storage implementation
    }
}
