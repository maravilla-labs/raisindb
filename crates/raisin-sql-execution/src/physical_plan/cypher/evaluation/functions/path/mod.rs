//! Path-finding functions for Cypher
//!
//! Provides functions for finding shortest paths, all paths, and distances
//! between nodes using BFS, A*, and Yen's algorithm.
//!
//! # Module Structure
//!
//! - `helpers` - Node extraction, adjacency graph building, serialization

mod helpers;

use std::collections::HashMap;

use raisin_cypher_parser::Expr;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;

use helpers::{build_adjacency_graph, extract_node_id_workspace, path_info_to_property_value};

use super::super::expr::evaluate_expr_async_impl;
use super::traits::FunctionContext;
use crate::physical_plan::cypher::types::VariableBinding;

/// shortestPath(start, end, maxDepth?) - Find shortest path between nodes
///
/// Uses BFS to find the shortest path. Returns path object or empty object if no path exists.
pub async fn evaluate_shortest_path<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::Validation(
            "shortestPath() requires 2-3 arguments (startNode, endNode, maxDepth?)".to_string(),
        ));
    }

    let (start_id, start_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;
    let (end_id, end_workspace) = extract_node_id_workspace(&args[1], binding, context).await?;

    let max_depth = if args.len() == 3 {
        match evaluate_expr_async_impl(&args[2], binding, context).await? {
            PropertyValue::Integer(n) => n as u32,
            PropertyValue::Float(n) => n as u32,
            _ => {
                return Err(Error::Validation(
                    "shortestPath() third argument (maxDepth) must be a number".to_string(),
                ))
            }
        }
    } else {
        10
    };

    tracing::debug!(
        "   shortestPath({}:{} -> {}:{}, maxDepth={})",
        start_workspace,
        start_id,
        end_workspace,
        end_id,
        max_depth
    );

    let adjacency = build_adjacency_graph(context).await?;
    let start_key = (start_workspace, start_id);
    let end_key = (end_workspace, end_id);

    let path = crate::physical_plan::cypher::algorithms::shortest_path(
        &adjacency, &start_key, &end_key, max_depth,
    );

    match path {
        Some(path_info) => {
            tracing::debug!("   Found path with length {}", path_info.length);
            Ok(path_info_to_property_value(&path_info))
        }
        None => {
            tracing::debug!("   No path found");
            Ok(PropertyValue::Object(HashMap::new()))
        }
    }
}

/// allShortestPaths(start, end, maxDepth?) - Find all shortest paths between nodes
///
/// Returns array of path objects with minimum length.
pub async fn evaluate_all_shortest_paths<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::Validation(
            "allShortestPaths() requires 2-3 arguments (startNode, endNode, maxDepth?)".to_string(),
        ));
    }

    let (start_id, start_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;
    let (end_id, end_workspace) = extract_node_id_workspace(&args[1], binding, context).await?;

    let max_depth = if args.len() == 3 {
        match evaluate_expr_async_impl(&args[2], binding, context).await? {
            PropertyValue::Integer(n) => n as u32,
            PropertyValue::Float(n) => n as u32,
            _ => {
                return Err(Error::Validation(
                    "allShortestPaths() third argument (maxDepth) must be a number".to_string(),
                ))
            }
        }
    } else {
        10
    };

    tracing::debug!(
        "   allShortestPaths({}:{} -> {}:{}, maxDepth={})",
        start_workspace,
        start_id,
        end_workspace,
        end_id,
        max_depth
    );

    let adjacency = build_adjacency_graph(context).await?;
    let start_key = (start_workspace, start_id);
    let end_key = (end_workspace, end_id);

    let paths = crate::physical_plan::cypher::algorithms::all_shortest_paths(
        &adjacency, &start_key, &end_key, max_depth, 100,
    );

    tracing::debug!("   Found {} shortest paths", paths.len());

    let path_array: Vec<PropertyValue> = paths.iter().map(path_info_to_property_value).collect();

    Ok(PropertyValue::Array(path_array))
}

/// astar(start, end, config?) - Find shortest path using A*
///
/// Uses A* algorithm with optional configuration for cost and heuristic.
pub async fn evaluate_astar<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::Validation(
            "astar() requires 2-3 arguments (startNode, endNode, config?)".to_string(),
        ));
    }

    let (start_id, start_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;
    let (end_id, end_workspace) = extract_node_id_workspace(&args[1], binding, context).await?;

    tracing::debug!(
        "   astar({}:{} -> {}:{})",
        start_workspace,
        start_id,
        end_workspace,
        end_id
    );

    let adjacency = build_adjacency_graph(context).await?;
    let start_key = (start_workspace, start_id);
    let end_key = (end_workspace, end_id);

    let path = crate::physical_plan::cypher::algorithms::astar_shortest_path(
        &adjacency,
        &start_key,
        &end_key,
        |_, _, _| 1.0,
        |_| 0.0,
    );

    match path {
        Some(path_info) => {
            tracing::debug!("   Found path with length {}", path_info.length);
            Ok(path_info_to_property_value(&path_info))
        }
        None => {
            tracing::debug!("   No path found");
            Ok(PropertyValue::Object(HashMap::new()))
        }
    }
}

/// kShortestPaths(start, end, k, config?) - Find K shortest paths between two nodes
///
/// Uses Yen's algorithm to find the K shortest loopless paths.
pub async fn evaluate_k_shortest_paths<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() < 3 || args.len() > 4 {
        return Err(Error::Validation(
            "kShortestPaths() requires 3-4 arguments (startNode, endNode, k, config?)".to_string(),
        ));
    }

    let (start_id, start_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;
    let (end_id, end_workspace) = extract_node_id_workspace(&args[1], binding, context).await?;

    let k = match evaluate_expr_async_impl(&args[2], binding, context).await? {
        PropertyValue::Integer(n) => n as usize,
        PropertyValue::Float(n) => n as usize,
        _ => {
            return Err(Error::Validation(
                "kShortestPaths() third argument (k) must be a number".to_string(),
            ))
        }
    };

    tracing::debug!(
        "   kShortestPaths({}:{} -> {}:{}, k={})",
        start_workspace,
        start_id,
        end_workspace,
        end_id,
        k
    );

    let adjacency = build_adjacency_graph(context).await?;
    let start_key = (start_workspace, start_id);
    let end_key = (end_workspace, end_id);

    let paths = crate::physical_plan::cypher::algorithms::k_shortest_paths(
        &adjacency,
        &start_key,
        &end_key,
        k,
        |_, _, _| 1.0,
    );

    tracing::debug!("   Found {} paths", paths.len());

    let path_array: Vec<PropertyValue> = paths.iter().map(path_info_to_property_value).collect();

    Ok(PropertyValue::Array(path_array))
}

/// distance(start, end) - Get shortest path length between nodes
///
/// Returns the number of hops in the shortest path, or -1 if no path exists.
pub async fn evaluate_distance<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    context: &FunctionContext<'_, S>,
) -> Result<PropertyValue, Error> {
    if args.len() != 2 {
        return Err(Error::Validation(
            "distance() requires exactly 2 arguments (startNode, endNode)".to_string(),
        ));
    }

    let (start_id, start_workspace) = extract_node_id_workspace(&args[0], binding, context).await?;
    let (end_id, end_workspace) = extract_node_id_workspace(&args[1], binding, context).await?;

    tracing::debug!(
        "   distance({}:{} -> {}:{})",
        start_workspace,
        start_id,
        end_workspace,
        end_id
    );

    let adjacency = build_adjacency_graph(context).await?;
    let start_key = (start_workspace, start_id);
    let end_key = (end_workspace, end_id);

    let path = crate::physical_plan::cypher::algorithms::shortest_path(
        &adjacency, &start_key, &end_key, 100,
    );

    match path {
        Some(path_info) => {
            tracing::debug!("   Distance = {}", path_info.length);
            Ok(PropertyValue::Integer(path_info.length as i64))
        }
        None => {
            tracing::debug!("   No path found");
            Ok(PropertyValue::Integer(-1))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_path_functions_signatures() {
        // Ensures the function signatures compile correctly.
        // Full testing requires a mock storage implementation.
    }
}
