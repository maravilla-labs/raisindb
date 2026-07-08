//! Union Operator Execution
//!
//! Concatenates the row streams of several child plans in order.
//!
//! Used to expand `col IN (v1, v2, ...)` into a union of per-value indexed
//! equality scans. Because each distinct literal over a single column selects a
//! disjoint row set (a node has exactly one path, id, node_type, or value at a
//! given JSON key), the branches never overlap and no de-duplication is needed.

use super::executor::{execute_plan, ExecutionContext, ExecutionError, RowStream};
use super::operators::PhysicalPlan;
use async_stream::try_stream;
use futures::stream::StreamExt;
use raisin_error::Error;
use raisin_storage::Storage;

/// Execute a Union operator by chaining child streams sequentially.
pub async fn execute_union<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let inputs = match plan {
        PhysicalPlan::Union { inputs } => inputs,
        _ => return Err(Error::Validation("Invalid plan for union".to_string())),
    };

    // Execute each child plan eagerly to set up its (lazy) stream. This borrows
    // `ctx` only for the duration of setup; the resulting streams are owned and
    // moved into the returned stream, keeping it `'static`.
    let mut child_streams: Vec<RowStream> = Vec::with_capacity(inputs.len());
    for input in inputs {
        child_streams.push(execute_plan(input, ctx).await?);
    }

    Ok(Box::pin(try_stream! {
        for mut stream in child_streams {
            while let Some(row_result) = stream.next().await {
                yield row_result?;
            }
        }
    }))
}
