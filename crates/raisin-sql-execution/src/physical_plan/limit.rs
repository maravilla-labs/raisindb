//! Limit Operator Execution
//!
//! Limits the number of rows and applies offset.

use super::executor::{execute_plan, ExecutionContext, ExecutionError, RowStream};
use super::operators::PhysicalPlan;
use async_stream::try_stream;
use futures::stream::StreamExt;
use raisin_error::Error;
use raisin_storage::Storage;

/// Execute a Limit operator
///
/// This is a streaming operator that:
/// 1. Skips the first `offset` rows
/// 2. Yields at most `limit` rows
/// 3. Stops processing after limit is reached
///
/// # Algorithm
///
/// ```text
/// count = 0
/// for each row from input:
///     if count < offset:
///         count++
///         continue
///     if count >= offset + limit:
///         break
///     yield row
///     count++
/// ```
pub async fn execute_limit<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (input, limit, offset) = match plan {
        PhysicalPlan::Limit {
            input,
            limit,
            offset,
        } => (input.as_ref(), *limit, *offset),
        _ => return Err(Error::Validation("Invalid plan for limit".to_string())),
    };

    // Execute input plan
    let mut input_stream = execute_plan(input, ctx).await?;

    Ok(Box::pin(try_stream! {
        let mut count = 0usize;
        let mut yielded = 0usize;

        // Process rows from input
        while let Some(row_result) = input_stream.next().await {
            let row = row_result?;

            // Skip offset rows
            if count < offset {
                count += 1;
                continue;
            }

            // Stop after limit rows
            if yielded >= limit {
                break;
            }

            yield row;
            count += 1;
            yielded += 1;
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_plan::operators::ScanReason;
    use raisin_sql::logical_plan::TableSchema;
    use std::sync::Arc;

    #[test]
    fn test_limit_structure() {
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

        let limit = PhysicalPlan::Limit {
            input: Box::new(scan),
            limit: 10,
            offset: 5,
        };

        assert_eq!(limit.inputs().len(), 1);
    }

    #[test]
    fn test_limit_describe() {
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

        let limit = PhysicalPlan::Limit {
            input: Box::new(scan),
            limit: 10,
            offset: 5,
        };

        let desc = limit.describe();
        assert_eq!(desc, "Limit: limit=10, offset=5");
    }
}
