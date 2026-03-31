//! Required Column Computation
//!
//! Traverses a logical plan tree from top to bottom, collecting all column
//! references needed for query execution.

use std::collections::HashSet;

use super::column_refs::extract_column_refs;
use crate::logical_plan::LogicalPlan;

/// Compute required columns for a plan node and its subtree
///
/// This traverses the plan from top to bottom, collecting all column references
/// that are needed for query execution. This includes:
/// - Columns in SELECT expressions
/// - Columns in WHERE predicates
/// - Columns in ORDER BY expressions
/// - Columns in aggregate functions and GROUP BY
pub fn compute_required_columns(plan: &LogicalPlan) -> HashSet<String> {
    match plan {
        LogicalPlan::Scan { .. } | LogicalPlan::TableFunction { .. } => {
            // Leaf node - no requirements from children
            HashSet::new()
        }

        LogicalPlan::Project { input, exprs } => {
            // Collect columns from projection expressions
            let mut cols = HashSet::new();
            for proj in exprs {
                cols.extend(extract_column_refs(&proj.expr));
            }
            // Also include requirements from child operators
            cols.extend(compute_required_columns(input));
            cols
        }

        LogicalPlan::Filter { input, predicate } => {
            // Collect columns from all conjuncts in the filter
            let mut cols = HashSet::new();
            for conjunct in &predicate.conjuncts {
                cols.extend(extract_column_refs(conjunct));
            }
            // Also include requirements from child operators
            cols.extend(compute_required_columns(input));
            cols
        }

        LogicalPlan::Sort { input, sort_exprs } => {
            // Collect columns from sort expressions
            let mut cols = HashSet::new();
            for sort_expr in sort_exprs {
                cols.extend(extract_column_refs(&sort_expr.expr));
            }
            // Also include requirements from child operators
            cols.extend(compute_required_columns(input));
            cols
        }

        LogicalPlan::Limit { input, .. } => {
            // Limit doesn't add any column requirements
            compute_required_columns(input)
        }

        LogicalPlan::Aggregate {
            input,
            group_by,
            aggregates,
        } => {
            let mut cols = HashSet::new();
            // Collect columns from GROUP BY expressions
            for expr in group_by {
                cols.extend(extract_column_refs(expr));
            }
            // Collect columns from aggregate function arguments
            for agg in aggregates {
                for arg in &agg.args {
                    cols.extend(extract_column_refs(arg));
                }
                // Also collect columns from FILTER clause
                if let Some(ref filter_expr) = agg.filter {
                    let filter_cols = extract_column_refs(filter_expr);
                    tracing::error!("🔍 FILTER clause columns: {:?}", filter_cols);
                    cols.extend(filter_cols);
                }
            }
            // Also include requirements from child operators
            cols.extend(compute_required_columns(input));
            tracing::error!("🔍 Total required columns for Aggregate: {:?}", cols);
            cols
        }

        LogicalPlan::Join {
            left,
            right,
            condition,
            ..
        } => {
            let mut cols = HashSet::new();
            // Collect columns from join condition
            if let Some(cond) = condition {
                cols.extend(extract_column_refs(cond));
            }
            // Also include requirements from both children
            cols.extend(compute_required_columns(left));
            cols.extend(compute_required_columns(right));
            cols
        }

        LogicalPlan::SemiJoin {
            left,
            right,
            left_key,
            right_key,
            ..
        } => {
            let mut cols = HashSet::new();
            // Collect columns from join keys
            cols.extend(extract_column_refs(left_key));
            cols.extend(extract_column_refs(right_key));
            // Also include requirements from both children
            cols.extend(compute_required_columns(left));
            cols.extend(compute_required_columns(right));
            cols
        }

        LogicalPlan::WithCTE { ctes, main_query } => {
            // Collect requirements from main query and all CTEs
            let mut cols = compute_required_columns(main_query);
            for (_, cte_plan) in ctes {
                cols.extend(compute_required_columns(cte_plan));
            }
            cols
        }

        LogicalPlan::CTEScan { .. } => {
            // CTE scans are leaf nodes - no column requirements to propagate
            HashSet::new()
        }

        LogicalPlan::Subquery { input, .. } => {
            // Collect requirements from the subquery's input plan
            compute_required_columns(input)
        }

        LogicalPlan::Distinct {
            input,
            distinct_spec,
        } => {
            let mut cols = compute_required_columns(input);
            // For DISTINCT ON, also include the DISTINCT ON expressions
            if let crate::logical_plan::operators::DistinctSpec::On(exprs) = distinct_spec {
                for expr in exprs {
                    cols.extend(extract_column_refs(expr));
                }
            }
            cols
        }

        LogicalPlan::LateralMap {
            input,
            function_expr,
            ..
        } => {
            let mut cols = HashSet::new();
            // Collect columns referenced by the lateral function expression
            cols.extend(extract_column_refs(function_expr));
            // Also include requirements from child operators
            cols.extend(compute_required_columns(input));
            cols
        }

        LogicalPlan::Window {
            input,
            window_exprs,
        } => {
            let mut cols = HashSet::new();
            // Collect columns from window expressions
            for window_expr in window_exprs {
                // Extract from window function arguments
                match &window_expr.function {
                    crate::analyzer::WindowFunction::Sum(expr)
                    | crate::analyzer::WindowFunction::Avg(expr)
                    | crate::analyzer::WindowFunction::Min(expr)
                    | crate::analyzer::WindowFunction::Max(expr) => {
                        cols.extend(extract_column_refs(expr));
                    }
                    _ => {}
                }
                // Extract from PARTITION BY
                for part_expr in &window_expr.partition_by {
                    cols.extend(extract_column_refs(part_expr));
                }
                // Extract from ORDER BY
                for (order_expr, _) in &window_expr.order_by {
                    cols.extend(extract_column_refs(order_expr));
                }
            }
            // Also include requirements from child operators
            cols.extend(compute_required_columns(input));
            cols
        }

        // DML operations - no column requirements to propagate (leaf nodes)
        LogicalPlan::Insert { .. }
        | LogicalPlan::Update { .. }
        | LogicalPlan::Delete { .. }
        | LogicalPlan::Order { .. }
        | LogicalPlan::Move { .. }
        | LogicalPlan::Copy { .. }
        | LogicalPlan::Translate { .. }
        | LogicalPlan::Relate { .. }
        | LogicalPlan::Unrelate { .. }
        | LogicalPlan::Empty => HashSet::new(),
    }
}
