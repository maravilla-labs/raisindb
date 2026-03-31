//! Window Function Executor
//!
//! Implements window function execution with support for:
//! - PARTITION BY: Divides rows into partitions
//! - ORDER BY: Sorts rows within each partition
//! - Window frames: ROWS BETWEEN and RANGE BETWEEN
//! - Ranking functions: ROW_NUMBER, RANK, DENSE_RANK
//! - Aggregate functions: COUNT, SUM, AVG, MIN, MAX
//!
//! # Algorithm
//!
//! Window function execution is a blocking operator that:
//! 1. Materializes all input rows (must see all data before computing)
//! 2. Evaluates PARTITION BY expressions to group rows into partitions
//! 3. Sorts each partition by ORDER BY expressions
//! 4. For each row in each partition:
//!    - Determines the window frame bounds for this row
//!    - Computes all window functions over the frame
//!    - Adds computed values to the output row
//! 5. Returns all rows with window function results appended
//!
//! # Performance Characteristics
//!
//! - Time complexity: O(n log n) for sorting within partitions
//! - Space complexity: O(n) for materializing all input rows
//! - Window frame computation: O(frame_size) per row

mod aggregates;
mod compare;
mod frame;
mod ranking;

#[cfg(test)]
mod tests;

use super::eval::eval_expr;
use super::executor::{execute_plan, ExecutionContext, ExecutionError, Row, RowStream};
use super::operators::PhysicalPlan;
use aggregates::{
    compute_avg_over_frame, compute_max_over_frame, compute_min_over_frame, compute_sum_over_frame,
};
use async_stream::try_stream;
use compare::{compare_literals, literal_to_property_value};
use frame::determine_frame_bounds;
use futures::stream::StreamExt;
use raisin_sql::analyzer::{Literal, WindowFunction};
use raisin_sql::logical_plan::WindowExpr;
use raisin_storage::Storage;
use ranking::RankState;
use std::cmp::Ordering;
use std::collections::HashMap;

/// Execute a window function operator
pub async fn execute_window<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (input, wes) = match plan {
        PhysicalPlan::Window {
            input,
            window_exprs,
        } => (input.as_ref(), window_exprs),
        _ => return Err(ExecutionError::Backend("Invalid plan".into())),
    };
    let s = execute_plan(input, ctx).await?;
    let rows: Vec<Row> = s
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    if rows.is_empty() {
        return Ok(Box::pin(futures::stream::iter(std::iter::empty::<
            Result<Row, ExecutionError>,
        >())));
    }
    let mut out = rows;
    for w in wes {
        out = compute_window_function(out, w)?;
    }
    Ok(Box::pin(try_stream! { for row in out { yield row; } }))
}

fn compute_window_function(rows: Vec<Row>, we: &WindowExpr) -> Result<Vec<Row>, ExecutionError> {
    let parts = partition_rows(rows, &we.partition_by)?;
    let mut result = Vec::new();
    for p in parts {
        result.extend(compute_window_for_partition(
            sort_partition(p, &we.order_by)?,
            we,
        )?);
    }
    Ok(result)
}

fn partition_rows(
    rows: Vec<Row>,
    pb: &[raisin_sql::analyzer::TypedExpr],
) -> Result<Vec<Vec<Row>>, ExecutionError> {
    if pb.is_empty() {
        return Ok(vec![rows]);
    }
    let mut map: HashMap<PartitionKey, Vec<Row>> = HashMap::new();
    for row in rows {
        map.entry(evaluate_partition_key(&row, pb)?)
            .or_default()
            .push(row);
    }
    Ok(map.into_values().collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PartitionKey {
    values: Vec<String>,
}

fn evaluate_partition_key(
    row: &Row,
    pb: &[raisin_sql::analyzer::TypedExpr],
) -> Result<PartitionKey, ExecutionError> {
    let mut vals = Vec::new();
    for e in pb {
        vals.push(format!("{:?}", eval_expr(e, row)?));
    }
    Ok(PartitionKey { values: vals })
}

fn sort_partition(
    part: Vec<Row>,
    ob: &[(raisin_sql::analyzer::TypedExpr, bool)],
) -> Result<Vec<Row>, ExecutionError> {
    if ob.is_empty() {
        return Ok(part);
    }
    let mut ev: Vec<(Row, Vec<Literal>)> = Vec::with_capacity(part.len());
    for row in part {
        let mut sv = Vec::with_capacity(ob.len());
        for (expr, _) in ob {
            sv.push(eval_expr(expr, &row)?);
        }
        ev.push((row, sv));
    }
    ev.sort_by(|a, b| {
        for (i, (_, desc)) in ob.iter().enumerate() {
            let c = compare_literals(&a.1[i], &b.1[i]);
            if c != Ordering::Equal {
                return if *desc { c.reverse() } else { c };
            }
        }
        Ordering::Equal
    });
    Ok(ev.into_iter().map(|(r, _)| r).collect())
}

fn compute_window_for_partition(
    part: Vec<Row>,
    we: &WindowExpr,
) -> Result<Vec<Row>, ExecutionError> {
    let n = part.len();
    let mut result = Vec::with_capacity(n);
    let mut rank_state = RankState::new();
    let input_rows = part.clone();
    for (idx, mut row) in part.into_iter().enumerate() {
        let (start, end) = determine_frame_bounds(idx, n, &we.frame);
        let val = match &we.function {
            WindowFunction::RowNumber => Literal::BigInt((idx + 1) as i64),
            WindowFunction::Rank => rank_state.compute_rank(idx, &result, we),
            WindowFunction::DenseRank => rank_state.compute_dense_rank(idx, &result, we),
            WindowFunction::Count => Literal::BigInt((end - start) as i64),
            WindowFunction::Sum(x) => compute_sum_over_frame(&input_rows, start, end, x)?,
            WindowFunction::Avg(x) => compute_avg_over_frame(&input_rows, start, end, x)?,
            WindowFunction::Min(x) => compute_min_over_frame(&input_rows, start, end, x)?,
            WindowFunction::Max(x) => compute_max_over_frame(&input_rows, start, end, x)?,
        };
        row.insert(we.alias.clone(), literal_to_property_value(val)?);
        result.push(row);
    }
    Ok(result)
}
