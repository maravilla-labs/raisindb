//! Generic closure-based projection functions
//!
//! Free-standing projection functions that accept a closure for expression evaluation,
//! providing a flexible API that does not require a specific storage type.

use raisin_cypher_parser::Expr;
use raisin_models::nodes::properties::PropertyValue;
use std::collections::{HashMap, HashSet};

use super::super::super::types::{CypherRow, VariableBinding};
use super::super::super::utils::{compute_property_value_hash, extract_column_name};
use super::super::accumulator::Accumulator;
use super::super::grouping::GroupKey;
use super::contains_aggregate;
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Project bindings to result rows based on RETURN clause
///
/// This is the main entry point for projection. It automatically detects whether
/// aggregation is needed and delegates to the appropriate implementation.
///
/// # Type Parameters
/// * `F` - Async function for evaluating expressions
///
/// # Arguments
/// * `bindings` - Input variable bindings to project
/// * `return_items` - RETURN clause items specifying what to project
/// * `evaluate_expr` - Async function to evaluate expressions in context of a binding
pub async fn project_bindings_to_rows<F, Fut>(
    bindings: &[VariableBinding],
    return_items: &[raisin_cypher_parser::ReturnItem],
    evaluate_expr: F,
) -> Result<Vec<CypherRow>>
where
    F: Fn(&Expr, &VariableBinding) -> Fut,
    Fut: std::future::Future<Output = Result<PropertyValue>>,
{
    // Fast path: check for aggregates
    let has_aggregates = return_items
        .iter()
        .any(|item| contains_aggregate(&item.expr));

    if has_aggregates {
        // Aggregation path
        project_with_aggregation(bindings, return_items, evaluate_expr).await
    } else {
        // Non-aggregation path (current behavior)
        project_without_aggregation(bindings, return_items, evaluate_expr).await
    }
}

/// Project bindings without aggregation (one binding -> one row)
///
/// Each binding produces exactly one output row. This is the simpler case
/// where no grouping or accumulation is needed.
async fn project_without_aggregation<F, Fut>(
    bindings: &[VariableBinding],
    return_items: &[raisin_cypher_parser::ReturnItem],
    evaluate_expr: F,
) -> Result<Vec<CypherRow>>
where
    F: Fn(&Expr, &VariableBinding) -> Fut,
    Fut: std::future::Future<Output = Result<PropertyValue>>,
{
    use raisin_cypher_parser::Expr;

    let mut rows = Vec::new();

    for (binding_idx, binding) in bindings.iter().enumerate() {
        let mut columns = Vec::with_capacity(return_items.len());
        let mut values = Vec::with_capacity(return_items.len());

        tracing::debug!(
            "   Binding #{}: nodes={:?}, rels={:?}",
            binding_idx + 1,
            binding.nodes.keys().collect::<Vec<_>>(),
            binding.relationships.keys().collect::<Vec<_>>()
        );

        for item in return_items {
            // Get column name (use alias if provided, otherwise extract from expression)
            let column_name = if let Some(alias) = &item.alias {
                alias.clone()
            } else {
                // Extract meaningful name from expression
                match &item.expr {
                    Expr::Variable(name) => name.clone(),
                    Expr::Property { expr, property } => {
                        // Use "var_prop" format (e.g., "source_id", "node_workspace")
                        if let Expr::Variable(var) = &**expr {
                            format!("{}_{}", var, property)
                        } else {
                            property.clone()
                        }
                    }
                    _ => "result".to_string(), // Default name for complex expressions
                }
            };

            // Evaluate the expression using the provided async function
            tracing::debug!("   Evaluating expression: {:?}", item.expr);
            let value = evaluate_expr(&item.expr, binding).await?;

            columns.push(column_name.clone());
            values.push(value);
            tracing::debug!("   Added '{}' to ordered result", column_name);
        }

        tracing::debug!(
            "   Row #{}: projected {} columns",
            binding_idx + 1,
            columns.len()
        );
        rows.push(CypherRow { columns, values });
    }

    Ok(rows)
}

/// Project bindings with aggregation (grouping + aggregates)
///
/// Groups bindings by non-aggregate expressions and computes aggregate functions
/// over each group. This is the more complex case involving accumulators and grouping.
async fn project_with_aggregation<F, Fut>(
    bindings: &[VariableBinding],
    return_items: &[raisin_cypher_parser::ReturnItem],
    evaluate_expr: F,
) -> Result<Vec<CypherRow>>
where
    F: Fn(&Expr, &VariableBinding) -> Fut,
    Fut: std::future::Future<Output = Result<PropertyValue>>,
{
    use raisin_cypher_parser::Expr;
    use raisin_models::nodes::properties::PropertyValue;

    tracing::debug!("   Using aggregation path");

    // Step 1: Identify grouping keys vs aggregates (single pass)
    let mut group_key_indices = Vec::new();
    let mut aggregate_indices = Vec::new();

    for (i, item) in return_items.iter().enumerate() {
        if contains_aggregate(&item.expr) {
            aggregate_indices.push(i);
        } else {
            group_key_indices.push(i);
        }
    }

    tracing::debug!(
        "   Grouping keys: {} indices, Aggregates: {} indices",
        group_key_indices.len(),
        aggregate_indices.len()
    );

    // Estimate capacity for accumulators
    let estimated_group_size = if group_key_indices.is_empty() {
        bindings.len() // Single group: collect all
    } else {
        bindings.len() / group_key_indices.len().max(1)
    };

    // Step 2: Build hash table with estimated capacity
    let mut groups: HashMap<GroupKey, (Vec<PropertyValue>, Vec<Accumulator>)> =
        HashMap::with_capacity(estimated_group_size.min(100));

    // Step 3: Single-pass grouping and accumulation
    for binding in bindings {
        // Evaluate group key (only non-aggregate expressions)
        let (group_key, group_values) = if group_key_indices.is_empty() {
            (GroupKey::Empty, Vec::new()) // Single group
        } else {
            let mut key_values = Vec::with_capacity(group_key_indices.len());
            let mut key_hashes = Vec::with_capacity(group_key_indices.len());
            for &i in &group_key_indices {
                let value = evaluate_expr(&return_items[i].expr, binding).await?;
                key_hashes.push(compute_property_value_hash(&value));
                key_values.push(value);
            }
            (GroupKey::Hashed(key_hashes), key_values)
        };

        // Get or create accumulators for this group
        let (_stored_group_values, accumulators) =
            groups.entry(group_key.clone()).or_insert_with(|| {
                let accs = return_items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        if aggregate_indices.contains(&i) {
                            if let Expr::FunctionCall { name, distinct, .. } = &item.expr {
                                Accumulator::new(name, *distinct, estimated_group_size)
                            } else {
                                Accumulator::None
                            }
                        } else {
                            Accumulator::None
                        }
                    })
                    .collect();

                (group_values.clone(), accs)
            });

        // Update accumulators (evaluate aggregate args only)
        for &agg_idx in &aggregate_indices {
            let item = &return_items[agg_idx];
            if let Expr::FunctionCall { args, .. } = &item.expr {
                let value = if args.is_empty() {
                    // COUNT(*) - no evaluation needed
                    PropertyValue::Integer(1)
                } else {
                    evaluate_expr(&args[0], binding).await?
                };
                accumulators[agg_idx].update(value)?;
            }
        }
    }

    tracing::debug!("   Created {} groups", groups.len());

    // Precompute lookup tables for deterministic ordering
    let group_index_lookup: HashMap<usize, usize> = group_key_indices
        .iter()
        .enumerate()
        .map(|(pos, &idx)| (idx, pos))
        .collect();
    let aggregate_index_set: HashSet<usize> = aggregate_indices.iter().copied().collect();

    // Step 4: Finalize groups to rows
    let mut rows = Vec::with_capacity(groups.len());
    for (_group_key, (group_values, accumulators)) in groups {
        let mut columns = Vec::with_capacity(return_items.len());
        let mut values = Vec::with_capacity(return_items.len());

        for (idx, item) in return_items.iter().enumerate() {
            let col_name = extract_column_name(item);
            columns.push(col_name);

            if let Some(&value_idx) = group_index_lookup.get(&idx) {
                let value = group_values
                    .get(value_idx)
                    .cloned()
                    .unwrap_or(PropertyValue::Null);
                values.push(value);
            } else if aggregate_index_set.contains(&idx) {
                values.push(accumulators[idx].finalize()?);
            } else {
                values.push(PropertyValue::Null);
            }
        }

        rows.push(CypherRow { columns, values });
    }

    tracing::info!("   Aggregation complete: {} result rows", rows.len());

    Ok(rows)
}
