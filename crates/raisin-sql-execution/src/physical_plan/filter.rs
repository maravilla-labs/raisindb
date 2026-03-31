//! Filter Operator Execution
//!
//! Filters input rows based on boolean predicates.

use super::eval::eval_expr;
use super::executor::{execute_plan, ExecutionContext, ExecutionError, RowStream};
use super::operators::PhysicalPlan;
use async_stream::try_stream;
use futures::stream::StreamExt;
use raisin_error::Error;
use raisin_storage::Storage;

/// Execute a Filter operator
///
/// Evaluates filter predicates against each input row and yields only rows
/// where all predicates evaluate to true (CNF - Conjunctive Normal Form).
///
/// # Algorithm
///
/// ```text
/// for each row from input:
///     for each predicate:
///         if predicate(row) == false:
///             skip this row
///     yield row
/// ```
pub async fn execute_filter<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (input, predicates) = match plan {
        PhysicalPlan::Filter { input, predicates } => (input.as_ref(), predicates.clone()),
        _ => return Err(Error::Validation("Invalid plan for filter".to_string())),
    };

    // Execute input plan first
    let mut input_stream = execute_plan(input, ctx).await?;

    Ok(Box::pin(try_stream! {
        // Process each row from input
        while let Some(row_result) = input_stream.next().await {
            let row = row_result?;

            // Evaluate all predicates (they must all be true)
            let mut all_match = true;
            for predicate in &predicates {
                match eval_expr(predicate, &row) {
                    Ok(raisin_sql::analyzer::Literal::Boolean(true)) => {
                        // Predicate matched, continue
                        continue;
                    }
                    Ok(raisin_sql::analyzer::Literal::Boolean(false))
                    | Ok(raisin_sql::analyzer::Literal::Null) => {
                        // Predicate didn't match or returned NULL (treat NULL as no match)
                        all_match = false;
                        break;
                    }
                    Ok(other) => {
                        Err(Error::Validation(format!(
                            "Filter predicate must return boolean, got {:?}",
                            other
                        )))?;
                        unreachable!();
                    }
                    Err(e) => {
                        Err(e)?;
                        unreachable!();
                    }
                }
            }

            // If all predicates matched, yield the row
            if all_match {
                yield row;
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical_plan::executor::Row;
    use crate::physical_plan::operators::ScanReason;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
    use raisin_sql::logical_plan::TableSchema;
    use std::sync::Arc;

    #[test]
    fn test_filter_predicate_structure() {
        // Test that we can construct a filter plan
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

        let predicate = TypedExpr::new(Expr::Literal(Literal::Boolean(true)), DataType::Boolean);

        let filter = PhysicalPlan::Filter {
            input: Box::new(scan),
            predicates: vec![predicate],
        };

        assert_eq!(filter.inputs().len(), 1);
    }

    #[test]
    fn test_filter_describe() {
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

        let filter = PhysicalPlan::Filter {
            input: Box::new(scan),
            predicates: vec![TypedExpr::literal(Literal::Boolean(true))],
        };

        let desc = filter.describe();
        assert_eq!(desc, "Filter: 1 predicates");
    }
}
