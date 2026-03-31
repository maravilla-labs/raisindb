//! Projection Operator Execution
//!
//! Computes projection expressions and outputs selected columns.

use super::eval::eval_expr_async;
use super::executor::{execute_plan, ExecutionContext, ExecutionError, Row, RowStream};
use super::operators::PhysicalPlan;
use super::types::to_property_value;
use async_stream::try_stream;
use futures::stream::StreamExt;
use raisin_error::Error;
use raisin_sql::analyzer::{Expr, Literal};
use raisin_storage::Storage;
use std::collections::HashMap;

/// Generate a cache key for an expression
///
/// This creates a stable key for caching expression results within a row.
/// Currently focuses on column references and JSON extractions.
fn expr_cache_key(expr: &Expr) -> Option<String> {
    match expr {
        // Cache column references: "table.column"
        Expr::Column { table, column } => Some(format!("{}.{}", table, column)),
        _ => None, // Don't cache other expression types for now
    }
}

/// Execute a Project operator
///
/// Evaluates projection expressions for each input row and creates a new row
/// with the computed values. This handles:
/// - Column references (pass-through)
/// - Computed expressions (DEPTH(path), JSON operators, etc.)
/// - Function calls
/// - Aliasing
///
/// # Algorithm
///
/// ```text
/// for each row from input:
///     new_row = {}
///     for each projection_expr:
///         value = eval(projection_expr.expr, row)
///         new_row[projection_expr.alias] = value
///     yield new_row
/// ```
pub async fn execute_project<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (input, exprs) = match plan {
        PhysicalPlan::Project { input, exprs } => (input.as_ref(), exprs.clone()),
        _ => return Err(Error::Validation("Invalid plan for project".to_string())),
    };

    tracing::debug!(num_exprs = exprs.len(), "project_row started");

    // Execute input plan first
    let mut input_stream = execute_plan(input, ctx).await?;

    // Clone ctx for the stream closure
    let ctx_clone = ctx.clone();

    Ok(Box::pin(try_stream! {
        // Process each row from input
        while let Some(row_result) = input_stream.next().await {
            let input_row = row_result?;
            let mut output_row = Row::new();
            let row_start = std::time::Instant::now();

            // Per-row expression cache to avoid redundant evaluations
            // This is particularly effective for repeated column references
            // like multiple JSON extractions from the same column
            let mut expr_cache: HashMap<String, Literal> = HashMap::new();

            // Evaluate each projection expression
            // Use async evaluator to handle EMBEDDING() and other async functions
            for proj_expr in &exprs {
                let expr_start = std::time::Instant::now();
                // Check if this expression can be cached and if it's already computed
                let value = if let Some(cache_key) = expr_cache_key(&proj_expr.expr.expr) {
                    if let Some(cached_value) = expr_cache.get(&cache_key) {
                        // Cache hit - reuse previously computed value
                        cached_value.clone()
                    } else {
                        // Cache miss - evaluate and store
                        let computed = eval_expr_async(&proj_expr.expr, &input_row, &ctx_clone).await?;
                        expr_cache.insert(cache_key, computed.clone());
                        computed
                    }
                } else {
                    // Expression not cacheable - evaluate normally
                    eval_expr_async(&proj_expr.expr, &input_row, &ctx_clone).await?
                };

                tracing::trace!(
                    alias = %proj_expr.alias,
                    elapsed_us = expr_start.elapsed().as_micros(),
                    cached = expr_cache.contains_key(&expr_cache_key(&proj_expr.expr.expr).unwrap_or_default()),
                    "Expression evaluated"
                );

                // Convert literal to PropertyValue
                let prop_value = match to_property_value(&value) {
                    Ok(pv) => pv,
                    Err(_) if matches!(value, raisin_sql::analyzer::Literal::Null) => {
                        // NULL values can be skipped or represented as absence
                        continue;
                    }
                    Err(e) => {
                        Err(Error::Validation(format!(
                            "Failed to convert expression result: {}",
                            e
                        )))?;
                        unreachable!();
                    }
                };

                output_row.insert(proj_expr.alias.clone(), prop_value);
            }

            tracing::trace!(
                num_exprs = exprs.len(),
                elapsed_us = row_start.elapsed().as_micros(),
                "Row projection completed"
            );

            // Cache is automatically dropped at end of row iteration

            yield output_row;
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_plan::operators::ScanReason;
    use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
    use raisin_sql::logical_plan::{ProjectionExpr, TableSchema};
    use std::sync::Arc;

    #[test]
    fn test_project_structure() {
        let scan = PhysicalPlan::TableScan {
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            workspace: "w1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            limit: None,
            reason: ScanReason::NoIndexAvailable,
        };

        let proj_expr = ProjectionExpr {
            expr: TypedExpr::column("nodes".to_string(), "id".to_string(), DataType::Text),
            alias: "id".to_string(),
        };

        let project = PhysicalPlan::Project {
            input: Box::new(scan),
            exprs: vec![proj_expr],
        };

        assert_eq!(project.inputs().len(), 1);
    }

    #[test]
    fn test_project_describe() {
        let scan = PhysicalPlan::TableScan {
            tenant_id: "t1".to_string(),
            repo_id: "r1".to_string(),
            branch: "main".to_string(),
            workspace: "w1".to_string(),
            table: "nodes".to_string(),
            alias: None,
            schema: Arc::new(TableSchema {
                table_name: "nodes".to_string(),
                columns: vec![],
            }),
            filter: None,
            projection: None,
            limit: None,
            reason: ScanReason::NoIndexAvailable,
        };

        let project = PhysicalPlan::Project {
            input: Box::new(scan),
            exprs: vec![ProjectionExpr {
                expr: TypedExpr::literal(Literal::Int(1)),
                alias: "one".to_string(),
            }],
        };

        let desc = project.describe();
        assert_eq!(desc, "Project: 1 expressions");
    }
}
