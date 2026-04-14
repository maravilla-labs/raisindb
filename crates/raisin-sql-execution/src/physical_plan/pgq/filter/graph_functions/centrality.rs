//! Centrality graph function evaluators (PageRank, Betweenness, Closeness, Degree)

use std::collections::HashMap;
use std::sync::Arc;

use raisin_sql::ast::Expr;
use raisin_storage::Storage;

use super::{build_adjacency, get_node_from_args, Result};
use crate::physical_plan::cypher::algorithms::{self, types::GraphNodeId};
use crate::physical_plan::pgq::context::PgqContext;
use crate::physical_plan::pgq::types::{SqlValue, VariableBinding};

pub(crate) async fn evaluate_pagerank<S: Storage>(
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "pagerank";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let config = algorithms::PageRankConfig::default();
    let ranks = algorithms::pagerank(&adjacency, &config);

    let mut results = std::collections::HashMap::new();
    for (node_id, &rank) in &ranks {
        results.insert(node_id.clone(), SqlValue::Float(rank));
    }
    context.set_cached_results(cache_key, results);

    match ranks.get(&node) {
        Some(&rank) => Ok(SqlValue::Float(rank)),
        None => Ok(SqlValue::Null),
    }
}

pub(crate) async fn evaluate_betweenness<S: Storage>(
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "betweenness";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let scores = algorithms::betweenness::all_betweenness_centrality(&adjacency);

    let mut results = std::collections::HashMap::new();
    for (node_id, &score) in &scores {
        results.insert(node_id.clone(), SqlValue::Float(score));
    }
    context.set_cached_results(cache_key, results);

    match scores.get(&node) {
        Some(&score) => Ok(SqlValue::Float(score)),
        None => Ok(SqlValue::Float(0.0)),
    }
}

pub(crate) async fn evaluate_closeness<S: Storage>(
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "closeness";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let scores = algorithms::centrality::all_closeness_centrality(&adjacency);

    let mut results = std::collections::HashMap::new();
    for (node_id, score) in &scores {
        results.insert(node_id.clone(), SqlValue::Float(*score));
    }
    context.set_cached_results(cache_key, results);

    let node_score = scores
        .iter()
        .find(|(n, _)| n == &node)
        .map(|(_, s)| *s)
        .unwrap_or(0.0);
    Ok(SqlValue::Float(node_score))
}

pub(crate) async fn evaluate_degree<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "degree_map";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let degrees = algorithms::centrality::all_degrees(&adjacency);

    let mut results = std::collections::HashMap::new();
    for (node_id, deg) in &degrees {
        results.insert(node_id.clone(), SqlValue::Integer(*deg as i64));
    }
    context.set_cached_results(cache_key, results);

    let node_degree = degrees
        .iter()
        .find(|(n, _)| n == &node)
        .map(|(_, d)| *d)
        .unwrap_or(0);
    Ok(SqlValue::Integer(node_degree as i64))
}

pub(crate) async fn evaluate_in_degree<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "in_degree_map";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;

    // Compute in-degree for all nodes in a single pass
    let mut in_degrees: HashMap<GraphNodeId, usize> = HashMap::new();
    for (source, neighbors) in adjacency.iter() {
        in_degrees.entry(source.clone()).or_insert(0);
        for (tgt_ws, tgt_id, _) in neighbors {
            *in_degrees
                .entry((tgt_ws.clone(), tgt_id.clone()))
                .or_insert(0) += 1;
        }
    }

    let mut results = std::collections::HashMap::new();
    for (n, &deg) in &in_degrees {
        results.insert(n.clone(), SqlValue::Integer(deg as i64));
    }
    context.set_cached_results(cache_key, results);

    let deg = in_degrees.get(&node).copied().unwrap_or(0);
    Ok(SqlValue::Integer(deg as i64))
}

pub(crate) async fn evaluate_out_degree<S: Storage>(
    args: &[Expr],
    binding: &VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "out_degree_map";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;

    // Compute out-degree for all nodes in a single pass
    let mut out_degrees: HashMap<GraphNodeId, usize> = HashMap::new();
    for (source, neighbors) in adjacency.iter() {
        out_degrees.insert(source.clone(), neighbors.len());
        for (tgt_ws, tgt_id, _) in neighbors {
            out_degrees
                .entry((tgt_ws.clone(), tgt_id.clone()))
                .or_insert(0);
        }
    }

    let mut results = std::collections::HashMap::new();
    for (n, &deg) in &out_degrees {
        results.insert(n.clone(), SqlValue::Integer(deg as i64));
    }
    context.set_cached_results(cache_key, results);

    let deg = out_degrees.get(&node).copied().unwrap_or(0);
    Ok(SqlValue::Integer(deg as i64))
}
