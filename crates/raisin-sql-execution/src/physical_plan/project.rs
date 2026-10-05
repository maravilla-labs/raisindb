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
use raisin_sql::logical_plan::ProjectionExpr;
use raisin_storage::Storage;

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
    let input_stream = execute_plan(input, ctx).await?;

    // Clone ctx for the stream closure
    let ctx_clone = ctx.clone();

    // A top-level RESOLVE is evaluated a chunk of rows at a time, so one
    // frontier walk (one batched read per level) serves the whole chunk.
    let resolve_at = super::project_resolve::chunked_resolve_exprs(&exprs);
    if ctx.batched_fetch && !resolve_at.is_empty() {
        return Ok(super::project_resolve::project_chunked(
            input_stream,
            exprs,
            resolve_at,
            ctx_clone,
        ));
    }

    // `SELECT *` / `SELECT a, b, c`: every expression a plain column. Each
    // row's values can then be MOVED into the output instead of cloned, when
    // no two expressions name the same input column.
    let all_columns = exprs
        .iter()
        .all(|e| matches!(e.expr.expr, Expr::Column { .. }));

    let mut input_stream = input_stream;
    Ok(Box::pin(try_stream! {
        // Process each row from input
        while let Some(row_result) = input_stream.next().await {
            let input_row = row_result?;
            let input_row = if all_columns {
                match move_columns(&exprs, input_row)? {
                    Ok(row) => {
                        yield row;
                        continue;
                    }
                    Err(unmoved) => unmoved,
                }
            } else {
                input_row
            };
            yield project_row(&exprs, &input_row, &ctx_clone, &mut []).await?;
        }
    }))
}

/// Project a row whose expressions are all plain columns by MOVING each
/// named value out of `input` — exactly the values [`project_row`] would
/// clone (same lookup, same conversion). `Ok(Err(input))` hands the row back
/// untouched when two expressions name the same column (a move would leave
/// the second one empty).
fn move_columns(exprs: &[ProjectionExpr], mut input: Row) -> Result<Result<Row, Row>, Error> {
    let mut slots: Vec<Option<usize>> = Vec::with_capacity(exprs.len());
    for proj_expr in exprs {
        let Expr::Column { table, column } = &proj_expr.expr.expr else {
            return Ok(Err(input));
        };
        let slot = super::eval::core::column_index(table, column, &input);
        if slot.is_some() && slots.contains(&slot) {
            return Ok(Err(input));
        }
        slots.push(slot);
    }
    let mut output = Row::with_capacity(exprs.len());
    for (proj_expr, slot) in exprs.iter().zip(slots) {
        let value = match slot {
            Some(i) => {
                let stored = std::mem::replace(
                    &mut input.columns[i],
                    raisin_models::nodes::properties::PropertyValue::Null,
                );
                super::project_value::projected_column_value_owned(stored).map_err(|e| {
                    Error::Validation(format!("Failed to convert column value: {}", e))
                })?
            }
            None => raisin_models::nodes::properties::PropertyValue::Null,
        };
        output.insert(proj_expr.alias.clone(), value);
    }
    Ok(Ok(output))
}

/// Project one row. `precomputed[i]`, when present, is the already evaluated
/// value of `exprs[i]` (a chunk-evaluated RESOLVE); it is taken, not cloned.
pub(crate) async fn project_row<S: Storage>(
    exprs: &[ProjectionExpr],
    input_row: &Row,
    ctx: &ExecutionContext<S>,
    precomputed: &mut [Option<raisin_models::nodes::properties::PropertyValue>],
) -> Result<Row, ExecutionError> {
    let mut output_row = Row::with_capacity(exprs.len());

    // Evaluate each projection expression
    // Use async evaluator to handle EMBEDDING() and other async functions
    for (i, proj_expr) in exprs.iter().enumerate() {
        // A value computed for the whole chunk (a top-level RESOLVE), already
        // the row's value.
        if let Some(value) = precomputed.get_mut(i).and_then(Option::take) {
            output_row.insert(proj_expr.alias.clone(), value);
            continue;
        }
        let value = if let Expr::Column { table, column } = &proj_expr.expr.expr {
            // A plain column passes its stored value through: the same value
            // `eval_column` + `to_property_value` produce, without converting
            // a property map to JSON and back (see `project_value`).
            let prop_value = match super::eval::core::column_value(table, column, input_row) {
                Some(stored) => {
                    super::project_value::projected_column_value(stored).map_err(|e| {
                        Error::Validation(format!("Failed to convert column value: {}", e))
                    })?
                }
                None => raisin_models::nodes::properties::PropertyValue::Null,
            };
            output_row.insert(proj_expr.alias.clone(), prop_value);
            continue;
        } else {
            eval_expr_async(&proj_expr.expr, input_row, ctx).await?
        };

        // Convert literal to PropertyValue
        let prop_value = match to_property_value(&value) {
            Ok(pv) => pv,
            Err(_) if matches!(value, Literal::Null) => {
                // NULL values can be skipped or represented as absence
                continue;
            }
            Err(e) => {
                return Err(Error::Validation(format!(
                    "Failed to convert expression result: {}",
                    e
                )));
            }
        };

        output_row.insert(proj_expr.alias.clone(), prop_value);
    }

    Ok(output_row)
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
            exprs: vec![proj_expr].into(),
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
            }]
            .into(),
        };

        let desc = project.describe();
        assert_eq!(desc, "Project: 1 expressions");
    }
}
