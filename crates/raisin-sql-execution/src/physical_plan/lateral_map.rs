//! LateralMap Operator Execution
//!
//! Evaluates a scalar function per input row and adds the result as a new column.
//! Used for `LATERAL function(args) AS alias` in FROM clause.

use super::eval::eval_expr_async;
use super::executor::{execute_plan, ExecutionContext, ExecutionError, RowStream};
use super::operators::PhysicalPlan;
use super::types::to_property_value;
use async_stream::try_stream;
use futures::stream::StreamExt;
use raisin_error::Error;
use raisin_storage::Storage;

/// Execute a LateralMap: for each input row, evaluate the function and add as new column
pub async fn execute_lateral_map<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (input, function_expr, column_name) = match plan {
        PhysicalPlan::LateralMap {
            input,
            function_expr,
            column_name,
        } => (input.as_ref(), function_expr.clone(), column_name.clone()),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for lateral_map".to_string(),
            ))
        }
    };

    let mut input_stream = execute_plan(input, ctx).await?;
    let ctx_clone = ctx.clone();

    Ok(Box::pin(try_stream! {
        while let Some(row_result) = input_stream.next().await {
            let mut row = row_result?;
            // Evaluate function with current row context (reuses existing async eval)
            let value = eval_expr_async(&function_expr, &row, &ctx_clone).await?;
            let prop_value = to_property_value(&value)
                .map_err(|e| Error::Internal(format!("LATERAL function result conversion failed: {}", e)))?;
            row.insert(column_name.clone(), prop_value);
            yield row;
        }
    }))
}
