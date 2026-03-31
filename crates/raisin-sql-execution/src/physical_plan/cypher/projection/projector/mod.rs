//! Projection logic for converting bindings to result rows
//!
//! This module contains functions for projecting variable bindings into Cypher result rows,
//! supporting both simple projections and aggregations with grouping.
//!
//! # Module Structure
//!
//! - `generic` - Free-standing generic projection functions (closure-based API)

mod generic;

pub use generic::project_bindings_to_rows;

use raisin_cypher_parser::{Expr, ReturnItem};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::Storage;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::super::evaluation::{evaluate_expr_async_impl, FunctionContext};
use super::super::types::{CypherRow, VariableBinding};
use super::super::utils::{compute_property_value_hash, extract_column_name};
use super::accumulator::Accumulator;
use super::grouping::GroupKey;
use crate::physical_plan::executor::ExecutionError;

type Result<T> = std::result::Result<T, ExecutionError>;

/// Check if a function name is an aggregate function
///
/// Returns true for: collect, count, sum, avg, min, max
pub(crate) fn is_aggregate_function(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "collect" | "count" | "sum" | "avg" | "min" | "max"
    )
}

/// Scan expression tree for aggregates (single pass)
///
/// Returns true if the expression contains any aggregate function call.
pub(crate) fn contains_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::FunctionCall { name, .. } => is_aggregate_function(name),
        Expr::Property { expr, .. } => contains_aggregate(expr),
        Expr::BinaryOp { left, right, .. } => contains_aggregate(left) || contains_aggregate(right),
        Expr::UnaryOp { expr, .. } => contains_aggregate(expr),
        _ => false,
    }
}

/// Projection engine for converting bindings to result rows
///
/// This struct encapsulates the execution context needed for projection
/// and provides a clean interface for projecting bindings to rows.
pub struct ProjectionEngine<S: Storage> {
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    workspace_id: String,
    revision: Option<raisin_hlc::HLC>,
    parameters: Arc<HashMap<String, PropertyValue>>,
}

impl<S: Storage> ProjectionEngine<S> {
    /// Create a new projection engine
    pub fn new(
        storage: Arc<S>,
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace_id: String,
        revision: Option<raisin_hlc::HLC>,
        parameters: Arc<HashMap<String, PropertyValue>>,
    ) -> Self {
        Self {
            storage,
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            revision,
            parameters,
        }
    }

    /// Project bindings to result rows based on RETURN clause
    ///
    /// This is the main entry point for projection. It automatically detects whether
    /// aggregation is needed and delegates to the appropriate implementation.
    pub async fn project(
        &self,
        bindings: &[VariableBinding],
        return_items: &[ReturnItem],
    ) -> Result<Vec<CypherRow>> {
        // Fast path: check for aggregates
        let has_aggregates = return_items
            .iter()
            .any(|item| contains_aggregate(&item.expr));

        if has_aggregates {
            // Aggregation path
            self.project_with_aggregation(bindings, return_items).await
        } else {
            // Non-aggregation path
            self.project_without_aggregation(bindings, return_items)
                .await
        }
    }

    /// Project bindings without aggregation (one binding -> one row)
    async fn project_without_aggregation(
        &self,
        bindings: &[VariableBinding],
        return_items: &[ReturnItem],
    ) -> Result<Vec<CypherRow>> {
        let mut rows = Vec::with_capacity(bindings.len());

        for (binding_idx, binding) in bindings.iter().enumerate() {
            if tracing::enabled!(tracing::Level::DEBUG) {
                tracing::debug!(
                    "   Binding #{}: nodes={:?}, rels={:?}",
                    binding_idx + 1,
                    binding.nodes.keys().collect::<Vec<_>>(),
                    binding.relationships.keys().collect::<Vec<_>>()
                );
            }

            let mut columns = Vec::with_capacity(return_items.len());
            let mut values = Vec::with_capacity(return_items.len());

            for item in return_items {
                // Get column name (use alias if provided, otherwise extract from expression)
                let column_name = if let Some(alias) = &item.alias {
                    alias.clone()
                } else {
                    // Extract meaningful name from expression
                    match &item.expr {
                        Expr::Variable(name) => name.clone(),
                        Expr::Property { expr, property } => {
                            if let Expr::Variable(var) = &**expr {
                                format!("{}_{}", var, property)
                            } else {
                                property.clone()
                            }
                        }
                        _ => "result".to_string(),
                    }
                };

                if tracing::enabled!(tracing::Level::DEBUG) {
                    tracing::debug!("   Evaluating expression: {:?}", item.expr);
                }
                let context = self.function_context();
                let value = evaluate_expr_async_impl(&item.expr, binding, &context).await?;

                columns.push(column_name.clone());
                values.push(value);
                if tracing::enabled!(tracing::Level::DEBUG) {
                    tracing::debug!("   Added '{}' to ordered result", column_name);
                }
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
    async fn project_with_aggregation(
        &self,
        bindings: &[VariableBinding],
        return_items: &[ReturnItem],
    ) -> Result<Vec<CypherRow>> {
        tracing::debug!("   Using aggregation path");

        // Step 1: Identify grouping keys vs aggregates
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
            bindings.len()
        } else {
            bindings.len() / group_key_indices.len().max(1)
        };

        // Step 2: Build hash table
        let mut groups: HashMap<GroupKey, (Vec<PropertyValue>, Vec<Accumulator>)> =
            HashMap::with_capacity(estimated_group_size.min(100));

        // Create evaluation context (reused for all bindings)
        let context = self.function_context();

        // Step 3: Single-pass grouping and accumulation
        for binding in bindings {
            // Evaluate group key
            let (group_key, group_values) = if group_key_indices.is_empty() {
                (GroupKey::Empty, Vec::new())
            } else {
                let mut key_values = Vec::with_capacity(group_key_indices.len());
                let mut key_hashes = Vec::with_capacity(group_key_indices.len());
                for &i in &group_key_indices {
                    let value =
                        evaluate_expr_async_impl(&return_items[i].expr, binding, &context).await?;
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

            // Update accumulators
            for &agg_idx in &aggregate_indices {
                let item = &return_items[agg_idx];
                if let Expr::FunctionCall { args, .. } = &item.expr {
                    let value = if args.is_empty() {
                        PropertyValue::Integer(1)
                    } else {
                        evaluate_expr_async_impl(&args[0], binding, &context).await?
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

    /// Create a FunctionContext for expression evaluation
    fn function_context(&self) -> FunctionContext<'_, S> {
        FunctionContext {
            storage: &*self.storage,
            tenant_id: &self.tenant_id,
            repo_id: &self.repo_id,
            branch: &self.branch,
            workspace_id: &self.workspace_id,
            revision: self.revision.as_ref(),
            parameters: &self.parameters,
        }
    }
}
