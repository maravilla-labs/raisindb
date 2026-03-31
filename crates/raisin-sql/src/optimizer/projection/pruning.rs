//! Projection Pruning Application
//!
//! NOTE: File intentionally exceeds 300 lines - the core function
//! `apply_projection_pruning_impl` is a single match expression over all
//! LogicalPlan variants that cannot be split without losing cohesion.
//!
//! Applies projection pushdown optimization to logical plans by computing
//! required columns and propagating them to Scan operators.

use std::collections::HashSet;
use std::sync::Arc;

use super::column_refs::extract_column_refs;
use crate::logical_plan::{LogicalPlan, TableSchema};

/// Get the output schema of a logical plan node
///
/// Returns the schema that this plan node produces, or None if schema cannot be determined.
fn get_plan_schema(plan: &LogicalPlan) -> Option<Arc<TableSchema>> {
    match plan {
        LogicalPlan::Scan { schema, .. } => Some(schema.clone()),
        LogicalPlan::TableFunction { schema, .. } => Some(schema.clone()),
        LogicalPlan::CTEScan { schema, .. } => Some(schema.clone()),
        LogicalPlan::Subquery { schema, .. } => Some(schema.clone()),
        LogicalPlan::Project { input, .. } => get_plan_schema(input),
        LogicalPlan::Filter { input, .. } => get_plan_schema(input),
        LogicalPlan::Sort { input, .. } => get_plan_schema(input),
        LogicalPlan::Limit { input, .. } => get_plan_schema(input),
        LogicalPlan::Distinct { input, .. } => get_plan_schema(input), // Distinct preserves schema
        LogicalPlan::Aggregate { .. } => None,                         // Aggregate changes schema
        LogicalPlan::Join { .. } => None,                              // Join combines schemas
        LogicalPlan::SemiJoin { left, .. } => get_plan_schema(left), // SemiJoin returns left schema
        LogicalPlan::WithCTE { main_query, .. } => get_plan_schema(main_query),
        LogicalPlan::Window { input, .. } => get_plan_schema(input), // Window adds columns to input schema
        LogicalPlan::LateralMap { input, .. } => get_plan_schema(input), // LateralMap adds a column to input schema
        // DML operations and empty plans don't have meaningful schemas for projection pruning
        LogicalPlan::Insert { .. }
        | LogicalPlan::Update { .. }
        | LogicalPlan::Delete { .. }
        | LogicalPlan::Order { .. }
        | LogicalPlan::Move { .. }
        | LogicalPlan::Copy { .. }
        | LogicalPlan::Translate { .. }
        | LogicalPlan::Relate { .. }
        | LogicalPlan::Unrelate { .. }
        | LogicalPlan::Empty => None,
    }
}

/// Apply projection pruning to a logical plan
///
/// This is the main entry point for projection pruning optimization.
/// It computes required columns and pushes them down to Scan operators.
pub fn apply_projection_pruning(plan: LogicalPlan) -> LogicalPlan {
    apply_projection_pruning_impl(plan, None)
}

/// Internal implementation that tracks required columns from parent operators
fn apply_projection_pruning_impl(
    plan: LogicalPlan,
    parent_requirements: Option<&HashSet<String>>,
) -> LogicalPlan {
    match plan {
        LogicalPlan::Scan {
            table,
            alias,
            schema,
            workspace,
            max_revision,
            branch_override,
            locales,
            filter,
            projection: _,
        } => {
            // Compute what columns are needed by parent operators
            let required = if let Some(reqs) = parent_requirements {
                reqs.clone()
            } else {
                // If no parent requirements, this is the root - include all columns
                schema
                    .columns
                    .iter()
                    .map(|c| c.name.clone())
                    .collect::<HashSet<_>>()
            };

            // Also include columns needed by any pushed-down filter
            let mut final_required = required;
            if let Some(ref filter_expr) = filter {
                final_required.extend(extract_column_refs(filter_expr));
            }

            // Expand wildcard markers (e.g., "table.*" from TO_JSON(table_alias))
            // Check if this scan's table/alias matches any wildcard markers
            let table_ref_name = alias.as_ref().unwrap_or(&table);
            let wildcard_marker = format!("{}.*", table_ref_name);

            if final_required.contains(&wildcard_marker) {
                // Remove the wildcard marker
                final_required.remove(&wildcard_marker);

                // Add all columns from this table's schema
                for col in &schema.columns {
                    final_required.insert(col.name.clone());
                }
            }

            // Convert to sorted vector for deterministic output
            let mut projection_vec: Vec<String> = final_required.into_iter().collect();
            projection_vec.sort();

            LogicalPlan::Scan {
                table,
                alias,
                schema,
                workspace,
                max_revision,
                branch_override,
                locales,
                filter,
                projection: Some(projection_vec),
            }
        }

        LogicalPlan::Filter { input, predicate } => {
            // Compute columns needed for this filter
            let mut filter_cols = HashSet::new();
            for conjunct in &predicate.conjuncts {
                filter_cols.extend(extract_column_refs(conjunct));
            }

            // Merge with parent requirements
            let mut child_requirements = parent_requirements.cloned().unwrap_or_default();
            child_requirements.extend(filter_cols);

            LogicalPlan::Filter {
                input: Box::new(apply_projection_pruning_impl(
                    *input,
                    Some(&child_requirements),
                )),
                predicate,
            }
        }

        LogicalPlan::Project { input, exprs } => {
            // Compute columns needed for projection expressions
            let mut proj_cols = HashSet::new();
            for proj in &exprs {
                proj_cols.extend(extract_column_refs(&proj.expr));
            }

            // Collect columns that are already in the projection expressions
            let mut existing_cols = HashSet::new();
            for proj in &exprs {
                existing_cols.extend(extract_column_refs(&proj.expr));
            }

            // Build new projection expressions including parent requirements
            let mut new_exprs = exprs.clone();

            // IMPORTANT: Also include parent requirements!
            // This handles cases like: SELECT id ... ORDER BY created_at
            // where ORDER BY references columns not in SELECT list.
            if let Some(parent_reqs) = parent_requirements {
                proj_cols.extend(parent_reqs.iter().cloned());

                // Add pass-through column references for parent requirements not in SELECT
                for col in parent_reqs {
                    if !existing_cols.contains(col) {
                        // Add a simple column reference to pass through this column
                        use crate::analyzer::{DataType, Expr, TypedExpr};
                        use crate::logical_plan::ProjectionExpr;
                        let col_expr = TypedExpr::new(
                            Expr::Column {
                                table: "".to_string(), // Unqualified - will be resolved
                                column: col.clone(),
                            },
                            DataType::Unknown, // Type will be inferred
                        );
                        new_exprs.push(ProjectionExpr {
                            expr: col_expr,
                            alias: col.clone(),
                        });
                    }
                }
            }

            LogicalPlan::Project {
                input: Box::new(apply_projection_pruning_impl(*input, Some(&proj_cols))),
                exprs: new_exprs,
            }
        }

        LogicalPlan::Sort { input, sort_exprs } => {
            // Compute columns needed for sort expressions
            let mut sort_cols = HashSet::new();
            for sort_expr in &sort_exprs {
                sort_cols.extend(extract_column_refs(&sort_expr.expr));
            }

            // Merge with parent requirements
            let mut child_requirements = parent_requirements.cloned().unwrap_or_default();
            child_requirements.extend(sort_cols);

            LogicalPlan::Sort {
                input: Box::new(apply_projection_pruning_impl(
                    *input,
                    Some(&child_requirements),
                )),
                sort_exprs,
            }
        }

        LogicalPlan::Limit {
            input,
            limit,
            offset,
        } => {
            // Limit doesn't add requirements, just pass through
            LogicalPlan::Limit {
                input: Box::new(apply_projection_pruning_impl(*input, parent_requirements)),
                limit,
                offset,
            }
        }

        LogicalPlan::Distinct {
            input,
            distinct_spec,
        } => {
            // Collect columns needed for DISTINCT ON expressions (if present)
            let mut distinct_cols = parent_requirements.cloned().unwrap_or_default();

            if let crate::logical_plan::operators::DistinctSpec::On(ref exprs) = distinct_spec {
                for expr in exprs {
                    distinct_cols.extend(extract_column_refs(expr));
                }
            }

            LogicalPlan::Distinct {
                input: Box::new(apply_projection_pruning_impl(*input, Some(&distinct_cols))),
                distinct_spec,
            }
        }

        LogicalPlan::Aggregate {
            input,
            group_by,
            aggregates,
        } => {
            // Compute columns needed for aggregate
            let mut agg_cols = HashSet::new();
            for expr in &group_by {
                agg_cols.extend(extract_column_refs(expr));
            }
            for agg in &aggregates {
                for arg in &agg.args {
                    agg_cols.extend(extract_column_refs(arg));
                }
                // Also collect columns from FILTER clause
                if let Some(ref filter_expr) = agg.filter {
                    let filter_cols = extract_column_refs(filter_expr);
                    tracing::error!(
                        "🔍 [apply_projection_pruning_impl] FILTER clause columns: {:?}",
                        filter_cols
                    );
                    agg_cols.extend(filter_cols);
                }
            }
            tracing::debug!(
                "🔍 [apply_projection_pruning_impl] Total columns for Aggregate: {:?}",
                agg_cols
            );

            LogicalPlan::Aggregate {
                input: Box::new(apply_projection_pruning_impl(*input, Some(&agg_cols))),
                group_by,
                aggregates,
            }
        }

        LogicalPlan::Join {
            left,
            right,
            join_type,
            condition,
        } => {
            // For joins, we need columns from:
            // 1. Join condition
            // 2. Parent requirements (if any)
            let mut required = parent_requirements.cloned().unwrap_or_default();

            if let Some(ref cond) = condition {
                required.extend(extract_column_refs(cond));
            }

            // Split requirements between left and right based on which side provides each column
            let left_schema = get_plan_schema(&left);
            let right_schema = get_plan_schema(&right);

            let mut left_required = HashSet::new();
            let mut right_required = HashSet::new();

            for col in required {
                // Check if column exists in left schema
                let in_left = left_schema
                    .as_ref()
                    .map(|s| s.columns.iter().any(|c| c.name == col))
                    .unwrap_or(false);

                // Check if column exists in right schema
                let in_right = right_schema
                    .as_ref()
                    .map(|s| s.columns.iter().any(|c| c.name == col))
                    .unwrap_or(false);

                if in_left {
                    left_required.insert(col.clone());
                }
                if in_right {
                    right_required.insert(col);
                }
            }

            LogicalPlan::Join {
                left: Box::new(apply_projection_pruning_impl(*left, Some(&left_required))),
                right: Box::new(apply_projection_pruning_impl(*right, Some(&right_required))),
                join_type,
                condition,
            }
        }

        LogicalPlan::SemiJoin {
            left,
            right,
            left_key,
            right_key,
            anti,
        } => {
            // For semi-joins, we need columns from:
            // 1. Left key and right key expressions
            // 2. Parent requirements (only from left, since SemiJoin returns left schema only)
            let mut left_required = parent_requirements.cloned().unwrap_or_default();

            // Add columns from left key
            left_required.extend(extract_column_refs(&left_key));

            // Right side only needs columns for the right key
            let mut right_required = HashSet::new();
            right_required.extend(extract_column_refs(&right_key));

            LogicalPlan::SemiJoin {
                left: Box::new(apply_projection_pruning_impl(*left, Some(&left_required))),
                right: Box::new(apply_projection_pruning_impl(*right, Some(&right_required))),
                left_key,
                right_key,
                anti,
            }
        }

        LogicalPlan::WithCTE { ctes, main_query } => {
            // Apply projection pruning to each CTE and the main query
            let pruned_ctes: Vec<(String, Box<LogicalPlan>)> = ctes
                .into_iter()
                .map(|(name, plan)| (name, Box::new(apply_projection_pruning_impl(*plan, None))))
                .collect();

            let pruned_main = apply_projection_pruning_impl(*main_query, parent_requirements);

            LogicalPlan::WithCTE {
                ctes: pruned_ctes,
                main_query: Box::new(pruned_main),
            }
        }

        LogicalPlan::CTEScan { .. } => {
            // CTE scans are already optimized (they reference materialized CTEs)
            plan
        }

        LogicalPlan::Subquery {
            input,
            alias,
            schema,
        } => {
            // Apply projection pruning to the subquery's input plan
            // The subquery itself is a boundary - we optimize its internal plan independently
            LogicalPlan::Subquery {
                input: Box::new(apply_projection_pruning_impl(*input, None)),
                alias,
                schema,
            }
        }

        LogicalPlan::Window {
            input,
            window_exprs,
        } => {
            // Compute columns needed for window expressions
            let mut window_cols = HashSet::new();
            for window_expr in &window_exprs {
                // Extract from window function arguments
                match &window_expr.function {
                    crate::analyzer::WindowFunction::Sum(expr)
                    | crate::analyzer::WindowFunction::Avg(expr)
                    | crate::analyzer::WindowFunction::Min(expr)
                    | crate::analyzer::WindowFunction::Max(expr) => {
                        window_cols.extend(extract_column_refs(expr));
                    }
                    _ => {}
                }
                // Extract from PARTITION BY
                for part_expr in &window_expr.partition_by {
                    window_cols.extend(extract_column_refs(part_expr));
                }
                // Extract from ORDER BY
                for (order_expr, _) in &window_expr.order_by {
                    window_cols.extend(extract_column_refs(order_expr));
                }
            }

            // Merge with parent requirements
            let mut child_requirements = parent_requirements.cloned().unwrap_or_default();
            child_requirements.extend(window_cols);

            LogicalPlan::Window {
                input: Box::new(apply_projection_pruning_impl(
                    *input,
                    Some(&child_requirements),
                )),
                window_exprs,
            }
        }

        LogicalPlan::LateralMap {
            input,
            function_expr,
            column_name,
        } => {
            // Compute columns needed for the lateral function expression
            let mut lateral_cols = HashSet::new();
            lateral_cols.extend(extract_column_refs(&function_expr));

            // Merge with parent requirements
            let mut child_requirements = parent_requirements.cloned().unwrap_or_default();
            child_requirements.extend(lateral_cols);

            LogicalPlan::LateralMap {
                input: Box::new(apply_projection_pruning_impl(
                    *input,
                    Some(&child_requirements),
                )),
                function_expr,
                column_name,
            }
        }

        LogicalPlan::TableFunction { .. } => plan,

        // DML operations and empty plans are leaf nodes - no projection pruning applies
        LogicalPlan::Insert { .. }
        | LogicalPlan::Update { .. }
        | LogicalPlan::Delete { .. }
        | LogicalPlan::Order { .. }
        | LogicalPlan::Move { .. }
        | LogicalPlan::Copy { .. }
        | LogicalPlan::Translate { .. }
        | LogicalPlan::Relate { .. }
        | LogicalPlan::Unrelate { .. }
        | LogicalPlan::Empty => plan,
    }
}
