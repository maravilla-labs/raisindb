//! Sort Operator Execution
//!
//! Sorts input rows by one or more sort expressions.
//! This is a blocking operator -- it must consume all input before producing output.
//!
//! # Module Structure
//!
//! - `comparison` - Literal ordering and row comparison
//! - `topn` - Heap-based TopN for small LIMIT queries

mod comparison;
mod topn;

use super::eval::{eval_expr, eval_expr_async};
use super::executor::{execute_plan, ExecutionContext, ExecutionError, Row, RowStream};
use super::operators::PhysicalPlan;
use async_stream::try_stream;
use comparison::compare_evaluated_rows;
use futures::stream::StreamExt;
use raisin_error::Error;
use raisin_sql::analyzer::Literal;
use raisin_storage::Storage;

/// Execute a Sort or TopN operator
///
/// Algorithm:
/// 1. Collects all rows from the input
/// 2. Pre-evaluates sort expressions for each row (handles EMBEDDING async calls)
/// 3. Sorts rows based on pre-evaluated values
/// 4. Yields sorted rows
///
/// For TopN with small limit (< 1000), uses heap-based O(N log K) optimization
/// instead of full O(N log N) sorting.
pub async fn execute_sort<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (input, sort_exprs, limit) = match plan {
        PhysicalPlan::Sort { input, sort_exprs } => {
            tracing::info!(
                "Executing Sort operator with {} sort expressions",
                sort_exprs.len()
            );
            (input.as_ref(), sort_exprs.clone(), None)
        }
        PhysicalPlan::TopN {
            input,
            sort_exprs,
            limit,
        } => {
            tracing::info!(
                "Executing TopN operator: {} sort expressions, limit={}",
                sort_exprs.len(),
                limit
            );
            (input.as_ref(), sort_exprs.clone(), Some(*limit))
        }
        _ => return Err(Error::Validation("Invalid plan for sort/topn".to_string())),
    };

    for (i, sort_expr) in sort_exprs.iter().enumerate() {
        tracing::debug!(
            "   Sort expr[{}]: {:?}, ascending={}",
            i,
            sort_expr.expr,
            sort_expr.ascending
        );
    }

    let input_stream = execute_plan(input, ctx).await?;

    // For TopN with small limit, use heap-based optimization
    if let Some(limit_val) = limit {
        if limit_val < 1000 {
            tracing::debug!("   Using heap optimization (limit={} < 1000)", limit_val);
            let sorted_rows =
                topn::execute_topn_with_heap(input_stream, &sort_exprs, limit_val, ctx).await?;

            return Ok(Box::pin(try_stream! {
                for row in sorted_rows {
                    yield row;
                }
            }));
        }
    }

    // Fall back to full sort for large limits or no limit
    let sorted_rows = full_sort(input_stream, &sort_exprs, limit, ctx).await?;

    Ok(Box::pin(try_stream! {
        for row in sorted_rows {
            yield row;
        }
    }))
}

/// Full sort: collect all rows, pre-evaluate, sort, and optionally truncate
async fn full_sort<S: Storage>(
    mut input_stream: RowStream,
    sort_exprs: &[raisin_sql::logical_plan::SortExpr],
    limit: Option<usize>,
    ctx: &ExecutionContext<S>,
) -> Result<Vec<Row>, ExecutionError> {
    let mut rows = Vec::new();

    while let Some(row_result) = input_stream.next().await {
        rows.push(row_result?);
    }

    tracing::info!("   Collected {} rows for sorting", rows.len());

    // Pre-evaluate sort expressions for all rows
    let mut evaluated_rows: Vec<(Row, Vec<Literal>)> = Vec::with_capacity(rows.len());

    for (row_idx, row) in rows.into_iter().enumerate() {
        let mut eval_values = Vec::with_capacity(sort_exprs.len());
        for sort_expr in sort_exprs {
            let value = eval_expr_async(&sort_expr.expr, &row, ctx).await?;

            if row_idx < 5 {
                tracing::debug!("   Row[{}] sort value: {:?}", row_idx, value);
            }

            eval_values.push(value);
        }
        evaluated_rows.push((row, eval_values));
    }

    tracing::debug!(
        "   Pre-evaluated sort expressions for {} rows",
        evaluated_rows.len()
    );

    // Sort by pre-evaluated values
    evaluated_rows.sort_by(|a, b| compare_evaluated_rows(&a.1, &b.1, sort_exprs));

    tracing::info!("   Sorting complete");

    // Apply limit if TopN
    if let Some(limit_val) = limit {
        let before_len = evaluated_rows.len();
        evaluated_rows.truncate(limit_val);
        tracing::info!(
            "   Applied TopN limit: {} -> {} rows",
            before_len,
            evaluated_rows.len()
        );
    }

    let sorted_rows: Vec<Row> = evaluated_rows.into_iter().map(|(row, _)| row).collect();
    tracing::info!("   Returning {} sorted rows", sorted_rows.len());

    Ok(sorted_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_plan::operators::ScanReason;
    use raisin_sql::analyzer::{DataType, Literal, TypedExpr};
    use raisin_sql::logical_plan::{SortExpr, TableSchema};
    use std::sync::Arc;

    #[test]
    fn test_sort_structure() {
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

        let sort_expr = SortExpr {
            expr: TypedExpr::column("nodes".to_string(), "name".to_string(), DataType::Text),
            ascending: true,
            nulls_first: false,
        };

        let sort = PhysicalPlan::Sort {
            input: Box::new(scan),
            sort_exprs: vec![sort_expr],
        };

        assert_eq!(sort.inputs().len(), 1);
    }
}
