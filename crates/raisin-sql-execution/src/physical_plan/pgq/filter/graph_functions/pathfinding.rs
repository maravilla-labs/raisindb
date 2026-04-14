//! Pathfinding graph function evaluators (BFS, SSSP)

use std::sync::Arc;

use raisin_sql::ast::Expr;
use raisin_storage::Storage;

use super::{
    build_adjacency, build_adjacency_with_weights, get_node_from_args, get_string_arg, Result,
};
use crate::physical_plan::cypher::algorithms;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::pgq::context::PgqContext;
use crate::physical_plan::pgq::types::{SqlValue, VariableBinding};

pub(crate) async fn evaluate_bfs<S: Storage>(
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let source_str = get_string_arg(args, 1).ok_or_else(|| {
        ExecutionError::Validation("bfs() requires second argument: source node ID".into())
    })?;

    let cache_key = format!("bfs:{}", source_str);

    if let Some(cached) = context.get_cached_result(&cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;

    let source = adjacency
        .keys()
        .find(|(_, id)| id == &source_str)
        .cloned()
        .unwrap_or_else(|| (node.0.clone(), source_str));

    let distances = algorithms::bfs_distances(&adjacency, &source);

    let mut results = std::collections::HashMap::new();
    for (node_id, &dist) in &distances {
        if dist == usize::MAX {
            results.insert(node_id.clone(), SqlValue::Null);
        } else {
            results.insert(node_id.clone(), SqlValue::Integer(dist as i64));
        }
    }
    context.set_cached_results(&cache_key, results);

    match distances.get(&node) {
        Some(&dist) if dist != usize::MAX => Ok(SqlValue::Integer(dist as i64)),
        _ => Ok(SqlValue::Null),
    }
}

pub(crate) async fn evaluate_sssp<S: Storage>(
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let source_str = get_string_arg(args, 1).ok_or_else(|| {
        ExecutionError::Validation("sssp() requires second argument: source node ID".into())
    })?;

    let cache_key = format!("sssp:{}", source_str);

    if let Some(cached) = context.get_cached_result(&cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let (adjacency, weights) = build_adjacency_with_weights(storage, context).await?;

    let source = adjacency
        .keys()
        .find(|(_, id)| id == &source_str)
        .cloned()
        .unwrap_or_else(|| (node.0.clone(), source_str));

    let distances = algorithms::sssp_distances_weighted(&adjacency, &source, &weights);

    let mut results = std::collections::HashMap::new();
    for (node_id, &dist) in &distances {
        results.insert(node_id.clone(), SqlValue::Float(dist));
    }
    context.set_cached_results(&cache_key, results);

    match distances.get(&node) {
        Some(&d) => Ok(SqlValue::Float(d)),
        None => Ok(SqlValue::Null),
    }
}
