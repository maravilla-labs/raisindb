//! Counting graph function evaluators (triangle_count, LCC)

use std::sync::Arc;

use raisin_sql::ast::Expr;
use raisin_storage::Storage;

use super::{build_adjacency, get_node_from_args, Result};
use crate::physical_plan::cypher::algorithms;
use crate::physical_plan::pgq::context::PgqContext;
use crate::physical_plan::pgq::types::{SqlValue, VariableBinding};

pub(crate) async fn evaluate_triangle_count<S: Storage>(
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "triangle_count";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let counts = algorithms::triangles::triangle_count(&adjacency);

    let mut results = std::collections::HashMap::new();
    for (node_id, &count) in &counts {
        results.insert(node_id.clone(), SqlValue::Integer(count as i64));
    }
    context.set_cached_results(cache_key, results);

    match counts.get(&node) {
        Some(&count) => Ok(SqlValue::Integer(count as i64)),
        None => Ok(SqlValue::Integer(0)),
    }
}

pub(crate) async fn evaluate_lcc<S: Storage>(
    args: &[Expr],
    binding: &mut VariableBinding,
    storage: &Arc<S>,
    context: &PgqContext,
) -> Result<SqlValue> {
    let node = get_node_from_args(args, binding)?;
    let cache_key = "lcc";

    if let Some(cached) = context.get_cached_result(cache_key, &node.0, &node.1) {
        return Ok(cached);
    }

    let adjacency = build_adjacency(storage, context).await?;
    let coefficients = algorithms::lcc::lcc(&adjacency);

    let mut results = std::collections::HashMap::new();
    for (node_id, &coeff) in &coefficients {
        results.insert(node_id.clone(), SqlValue::Float(coeff));
    }
    context.set_cached_results(cache_key, results);

    match coefficients.get(&node) {
        Some(&coeff) => Ok(SqlValue::Float(coeff)),
        None => Ok(SqlValue::Null),
    }
}
