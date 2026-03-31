//! Nested Loop Join Executor
//!
//! Implements the nested loop join algorithm with support for all join types.
//!
//! Algorithm:
//! 1. For each row from left input:
//!    2. For each row from right input:
//!   3. Evaluate join condition
//!   4. If match, output merged row
//!    5. For LEFT/FULL joins: output unmatched left row (no right columns)
//! 6. For RIGHT/FULL joins: output unmatched right rows (no left columns)
//!
//! Complexity: O(n * m) where n = left rows, m = right rows
//! Memory: O(m) - right side is materialized

use super::eval::eval_expr;
use super::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use super::operators::PhysicalPlan;
use futures::stream::{self, StreamExt};
use indexmap::IndexMap;
use raisin_sql::analyzer::{JoinType, Literal, TypedExpr};
use raisin_storage::Storage;

/// Execute a nested loop join
pub async fn execute_nested_loop_join<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (left, right, join_type, condition) = match plan {
        PhysicalPlan::NestedLoopJoin {
            left,
            right,
            join_type,
            condition,
        } => (left, right, join_type, condition),
        _ => {
            return Err(ExecutionError::Backend(
                "Invalid plan passed to execute_nested_loop_join".to_string(),
            ))
        }
    };

    // Execute both inputs
    let left_stream = super::executor::execute_plan(left.as_ref(), ctx).await?;
    let right_stream = super::executor::execute_plan(right.as_ref(), ctx).await?;

    // Materialize right side into a vector, handling errors
    let mut right_rows = Vec::new();
    let right_vec: Vec<_> = right_stream.collect().await;
    for row_result in right_vec {
        right_rows.push(row_result?);
    }

    // Create the join stream based on join type
    let output_rows = match join_type {
        JoinType::Cross => execute_cross_join(left_stream, right_rows).await?,
        JoinType::Inner => execute_inner_join(left_stream, right_rows, condition.clone()).await?,
        JoinType::Left => execute_left_join(left_stream, right_rows, condition.clone()).await?,
        JoinType::Right => execute_right_join(left_stream, right_rows, condition.clone()).await?,
        JoinType::Full => execute_full_join(left_stream, right_rows, condition.clone()).await?,
    };

    // Convert Vec<Row> to stream of Result<Row, ExecutionError>
    Ok(Box::pin(stream::iter(output_rows.into_iter().map(Ok))))
}

/// Execute CROSS JOIN (Cartesian product)
async fn execute_cross_join(
    mut left_stream: RowStream,
    right_rows: Vec<Row>,
) -> Result<Vec<Row>, ExecutionError> {
    let mut output = Vec::new();

    // For each left row
    while let Some(row_result) = left_stream.next().await {
        let left_row = row_result?;
        // For each right row
        for right_row in &right_rows {
            // Always merge (no condition for CROSS JOIN)
            output.push(merge_rows(&left_row, right_row));
        }
    }

    Ok(output)
}

/// Execute INNER JOIN
async fn execute_inner_join(
    mut left_stream: RowStream,
    right_rows: Vec<Row>,
    condition: Option<TypedExpr>,
) -> Result<Vec<Row>, ExecutionError> {
    let mut output = Vec::new();

    // For each left row
    while let Some(row_result) = left_stream.next().await {
        let left_row = row_result?;
        // For each right row
        for right_row in &right_rows {
            // Check join condition
            if evaluate_join_condition(&left_row, right_row, &condition) {
                // Merge and output
                output.push(merge_rows(&left_row, right_row));
            }
        }
    }

    Ok(output)
}

/// Execute LEFT OUTER JOIN
async fn execute_left_join(
    mut left_stream: RowStream,
    right_rows: Vec<Row>,
    condition: Option<TypedExpr>,
) -> Result<Vec<Row>, ExecutionError> {
    let mut output = Vec::new();

    // For each left row
    while let Some(row_result) = left_stream.next().await {
        let left_row = row_result?;
        let mut matched = false;

        // For each right row
        for right_row in &right_rows {
            // Check join condition
            if evaluate_join_condition(&left_row, right_row, &condition) {
                // Merge and output
                output.push(merge_rows(&left_row, right_row));
                matched = true;
            }
        }

        // If no match, output left row only (no right columns)
        if !matched {
            output.push(left_row);
        }
    }

    Ok(output)
}

/// Execute RIGHT OUTER JOIN
async fn execute_right_join(
    mut left_stream: RowStream,
    right_rows: Vec<Row>,
    condition: Option<TypedExpr>,
) -> Result<Vec<Row>, ExecutionError> {
    let mut output = Vec::new();
    let mut matched_right = vec![false; right_rows.len()];

    // For each left row
    while let Some(row_result) = left_stream.next().await {
        let left_row = row_result?;
        // For each right row
        for (idx, right_row) in right_rows.iter().enumerate() {
            // Check join condition
            if evaluate_join_condition(&left_row, right_row, &condition) {
                // Merge and output
                output.push(merge_rows(&left_row, right_row));
                matched_right[idx] = true;
            }
        }
    }

    // Output unmatched right rows (no left columns)
    for (idx, right_row) in right_rows.iter().enumerate() {
        if !matched_right[idx] {
            output.push(right_row.clone());
        }
    }

    Ok(output)
}

/// Execute FULL OUTER JOIN
async fn execute_full_join(
    mut left_stream: RowStream,
    right_rows: Vec<Row>,
    condition: Option<TypedExpr>,
) -> Result<Vec<Row>, ExecutionError> {
    let mut output = Vec::new();
    let mut matched_right = vec![false; right_rows.len()];

    // For each left row
    while let Some(row_result) = left_stream.next().await {
        let left_row = row_result?;
        let mut matched_left = false;

        // For each right row
        for (idx, right_row) in right_rows.iter().enumerate() {
            // Check join condition
            if evaluate_join_condition(&left_row, right_row, &condition) {
                // Merge and output
                output.push(merge_rows(&left_row, right_row));
                matched_left = true;
                matched_right[idx] = true;
            }
        }

        // If left row didn't match, output left row only
        if !matched_left {
            output.push(left_row);
        }
    }

    // Output unmatched right rows (no left columns)
    for (idx, right_row) in right_rows.iter().enumerate() {
        if !matched_right[idx] {
            output.push(right_row.clone());
        }
    }

    Ok(output)
}

/// Evaluate join condition on a pair of rows
fn evaluate_join_condition(left_row: &Row, right_row: &Row, condition: &Option<TypedExpr>) -> bool {
    match condition {
        None => true, // No condition = always match (CROSS JOIN)
        Some(expr) => {
            // Merge rows temporarily for evaluation
            let merged = merge_rows(left_row, right_row);

            // Evaluate the condition
            match eval_expr(expr, &merged) {
                Ok(Literal::Boolean(result)) => result,
                _ => false, // Non-boolean or error = no match
            }
        }
    }
}

/// Merge two rows into one
fn merge_rows(left: &Row, right: &Row) -> Row {
    let mut merged = IndexMap::new();

    // Add all left columns
    for (k, v) in &left.columns {
        merged.insert(k.clone(), v.clone());
    }

    // Add all right columns (may overwrite if same column name)
    for (k, v) in &right.columns {
        merged.insert(k.clone(), v.clone());
    }

    Row::from_map(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::PropertyValue;

    #[test]
    fn test_merge_rows() {
        let mut left_cols = IndexMap::new();
        left_cols.insert("id".to_string(), PropertyValue::Integer(1));
        left_cols.insert(
            "name".to_string(),
            PropertyValue::String("Alice".to_string()),
        );
        let left = Row::from_map(left_cols);

        let mut right_cols = IndexMap::new();
        right_cols.insert("id".to_string(), PropertyValue::Integer(2));
        right_cols.insert("city".to_string(), PropertyValue::String("NYC".to_string()));
        let right = Row::from_map(right_cols);

        let merged = merge_rows(&left, &right);

        // Right columns overwrite left when there's a conflict
        assert_eq!(merged.columns.len(), 3);
        assert_eq!(merged.get("id"), Some(&PropertyValue::Integer(2)));
        assert_eq!(
            merged.get("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
        assert_eq!(
            merged.get("city"),
            Some(&PropertyValue::String("NYC".to_string()))
        );
    }
}
