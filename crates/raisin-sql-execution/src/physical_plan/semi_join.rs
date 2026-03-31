//! Hash Semi-Join Executor
//!
//! Implements the hash semi-join algorithm for IN subquery support.
//!
//! Algorithm:
//! 1. Build Phase: Materialize right side and build hash set of distinct values
//! 2. Probe Phase: For each left row:
//!    - Evaluate left key expression
//!    - Check if key exists in hash set
//!    - For semi-join (IN): output row if key exists
//!    - For anti-join (NOT IN): output row if key doesn't exist
//!
//! Complexity: O(n + m) where n = left rows, m = right rows
//! Memory: O(distinct values in right side) - only unique values stored
//!
//! This is more efficient than a regular join because:
//! - We only store distinct values (not full rows)
//! - We don't need to merge rows from both sides
//! - Early termination: once we find a match, we're done with that left row

use super::eval::eval_expr;
use super::executor::{ExecutionContext, ExecutionError, RowStream};
use super::operators::PhysicalPlan;
use futures::stream::{self, StreamExt};
use raisin_sql::analyzer::Literal;
use raisin_storage::Storage;
use std::collections::HashSet;

/// Execute a hash semi-join
pub async fn execute_hash_semi_join<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (left, right, left_key, right_key, anti) = match plan {
        PhysicalPlan::HashSemiJoin {
            left,
            right,
            left_key,
            right_key,
            anti,
        } => (left, right, left_key, right_key, *anti),
        _ => {
            return Err(ExecutionError::Backend(
                "Invalid plan passed to execute_hash_semi_join".to_string(),
            ))
        }
    };

    // Execute right side first to build the hash set
    let right_stream = super::executor::execute_plan(right.as_ref(), ctx).await?;

    // Build Phase: Collect all distinct values from right side into a hash set
    let mut value_set: HashSet<String> = HashSet::new();
    let right_vec: Vec<_> = right_stream.collect().await;

    for row_result in right_vec {
        let row = row_result?;
        let value = eval_expr(right_key, &row)?;
        let key_string = value_to_hash_key(&value);
        value_set.insert(key_string);
    }

    tracing::debug!(
        "HashSemiJoin: Built hash set with {} distinct values, anti={}",
        value_set.len(),
        anti
    );

    // Execute left side
    let mut left_stream = super::executor::execute_plan(left.as_ref(), ctx).await?;

    // Probe Phase: For each left row, check if key exists in hash set
    let mut output_rows = Vec::new();

    while let Some(row_result) = left_stream.next().await {
        let row = row_result?;
        let value = eval_expr(left_key, &row)?;
        let key_string = value_to_hash_key(&value);

        let key_exists = value_set.contains(&key_string);

        // For semi-join (IN): output if key exists
        // For anti-join (NOT IN): output if key doesn't exist
        let should_output = if anti { !key_exists } else { key_exists };

        if should_output {
            output_rows.push(row);
        }
    }

    tracing::debug!(
        "HashSemiJoin: Produced {} output rows (anti={})",
        output_rows.len(),
        anti
    );

    // Convert Vec<Row> to stream of Result<Row, ExecutionError>
    Ok(Box::pin(stream::iter(output_rows.into_iter().map(Ok))))
}

/// Convert a literal value to a hash key string
///
/// Creates a string representation of the value for hashing.
/// This allows us to use a simple HashSet<String> without requiring
/// Literal to implement Hash.
fn value_to_hash_key(value: &Literal) -> String {
    match value {
        Literal::Null => "NULL".to_string(),
        Literal::Boolean(b) => format!("BOOL:{}", b),
        Literal::Int(i) => format!("INT:{}", i),
        Literal::BigInt(i) => format!("BIGINT:{}", i),
        Literal::Double(d) => format!("DOUBLE:{}", d),
        Literal::Text(s) => format!("TEXT:{}", s),
        Literal::Uuid(u) => format!("UUID:{}", u),
        Literal::Path(p) => format!("PATH:{}", p),
        Literal::Timestamp(t) => format!("TS:{}", t),
        Literal::Interval(d) => format!("INTERVAL:{}", d),
        Literal::JsonB(j) => format!("JSON:{}", j),
        Literal::Geometry(g) => format!("GEOM:{:?}", g),
        Literal::Vector(v) => format!("VEC:{:?}", v),
        Literal::Parameter(p) => format!("PARAM:{}", p),
    }
}
