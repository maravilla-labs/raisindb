//! Community detection graph function evaluators (WCC, CDLP, Louvain, community_id,
//! community_count, component_count)

use std::sync::Arc;

use raisin_sql::ast::Expr;
use raisin_storage::Storage;

use super::{build_adjacency, get_node_from_args, Result};
use crate::physical_plan::cypher::algorithms;
use crate::physical_plan::pgq::context::PgqContext;
use crate::physical_plan::pgq::types::{SqlValue, VariableBinding};

pub(crate) async fn evaluate_cdlp<S: Storage>(
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "cdlp";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let communities = algorithms::cdlp::cdlp(&adjacency, 10);

    let mut results = std::collections::HashMap::new();
    for (node_id, &community) in &communities {
        results.insert(node_id.clone(), SqlValue::Integer(community as i64));
    }
    context.set_cached_results(cache_key, results);

    match communities.get(&node) {
        Some(&community) => Ok(SqlValue::Integer(community as i64)),
        None => Ok(SqlValue::Null),
    }
}

pub(crate) async fn evaluate_wcc<S: Storage>(
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "wcc";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let components = algorithms::connected_components::connected_components(&adjacency);

    let mut results = std::collections::HashMap::new();
    for (node_id, component) in &components {
        results.insert(node_id.clone(), SqlValue::Integer(*component as i64));
    }
    context.set_cached_results(cache_key, results);

    match components.get(&node) {
        Some(&component) => Ok(SqlValue::Integer(component as i64)),
        None => Ok(SqlValue::Null),
    }
}

pub(crate) async fn evaluate_louvain<S: Storage>(
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "louvain";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let communities =
        algorithms::louvain::louvain(&adjacency, &algorithms::louvain::LouvainConfig::default());

    let mut results = std::collections::HashMap::new();
    for (node_id, &community) in &communities {
        results.insert(node_id.clone(), SqlValue::Integer(community as i64));
    }
    context.set_cached_results(cache_key, results);

    match communities.get(&node) {
        Some(&community) => Ok(SqlValue::Integer(community as i64)),
        None => Ok(SqlValue::Null),
    }
}

pub(crate) async fn evaluate_community_id<S: Storage>(
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "community_id";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let communities = algorithms::label_propagation::label_propagation(
        &adjacency,
        &algorithms::label_propagation::LabelPropagationConfig::default(),
    );

    let mut results = std::collections::HashMap::new();
    for (node_id, &community) in &communities {
        results.insert(node_id.clone(), SqlValue::Integer(community as i64));
    }
    context.set_cached_results(cache_key, results);

    match communities.get(&node) {
        Some(&community) => Ok(SqlValue::Integer(community as i64)),
        None => Ok(SqlValue::Null),
    }
}

pub(crate) async fn evaluate_community_count<S: Storage>(
    _args: &[Expr],
    _binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let cache_key = "community_count_result";

    if let Some(cached) = context.get_cached_result(cache_key, "__scalar__", "__scalar__") {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let count = algorithms::community_count(&adjacency);

    let mut results = std::collections::HashMap::new();
    results.insert(
        ("__scalar__".to_string(), "__scalar__".to_string()),
        SqlValue::Integer(count as i64),
    );
    context.set_cached_results(cache_key, results);

    Ok(SqlValue::Integer(count as i64))
}

pub(crate) async fn evaluate_component_count<S: Storage>(
    _args: &[Expr],
    _binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let cache_key = "component_count_result";

    if let Some(cached) = context.get_cached_result(cache_key, "__scalar__", "__scalar__") {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let count = algorithms::component_count(&adjacency);

    let mut results = std::collections::HashMap::new();
    results.insert(
        ("__scalar__".to_string(), "__scalar__".to_string()),
        SqlValue::Integer(count as i64),
    );
    context.set_cached_results(cache_key, results);

    Ok(SqlValue::Integer(count as i64))
}
