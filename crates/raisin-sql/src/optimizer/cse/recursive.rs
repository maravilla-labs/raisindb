//! Recursive CSE application across plan tree

use crate::logical_plan::LogicalPlan;

use super::apply::apply_cse;
use super::config::CseConfig;

/// Apply CSE optimization recursively to all Project nodes in the plan tree
///
/// This is a more aggressive version that applies CSE to every Project node
/// in the plan, not just the top-level one. Use this when you want to optimize
/// complex plans with multiple projection layers.
///
/// # Arguments
///
/// * `plan` - The logical plan to optimize
/// * `config` - CSE configuration
///
/// # Returns
///
/// An optimized plan with CSE applied to all applicable nodes.
pub fn apply_cse_recursive(plan: LogicalPlan, config: &CseConfig) -> LogicalPlan {
    match plan {
        LogicalPlan::Project { input, exprs } => {
            // First, recursively optimize the input
            let optimized_input = apply_cse_recursive(*input, config);

            // Then apply CSE to this projection
            let current_plan = LogicalPlan::Project {
                input: Box::new(optimized_input),
                exprs,
            };

            apply_cse(current_plan, config)
        }

        LogicalPlan::Filter { input, predicate } => LogicalPlan::Filter {
            input: Box::new(apply_cse_recursive(*input, config)),
            predicate,
        },

        LogicalPlan::Sort { input, sort_exprs } => LogicalPlan::Sort {
            input: Box::new(apply_cse_recursive(*input, config)),
            sort_exprs,
        },

        LogicalPlan::Limit {
            input,
            limit,
            offset,
        } => LogicalPlan::Limit {
            input: Box::new(apply_cse_recursive(*input, config)),
            limit,
            offset,
        },

        LogicalPlan::Distinct {
            input,
            distinct_spec,
        } => LogicalPlan::Distinct {
            input: Box::new(apply_cse_recursive(*input, config)),
            distinct_spec,
        },

        LogicalPlan::Aggregate {
            input,
            group_by,
            aggregates,
        } => LogicalPlan::Aggregate {
            input: Box::new(apply_cse_recursive(*input, config)),
            group_by,
            aggregates,
        },

        LogicalPlan::Join {
            left,
            right,
            join_type,
            condition,
        } => LogicalPlan::Join {
            left: Box::new(apply_cse_recursive(*left, config)),
            right: Box::new(apply_cse_recursive(*right, config)),
            join_type,
            condition,
        },

        LogicalPlan::SemiJoin {
            left,
            right,
            left_key,
            right_key,
            anti,
        } => LogicalPlan::SemiJoin {
            left: Box::new(apply_cse_recursive(*left, config)),
            right: Box::new(apply_cse_recursive(*right, config)),
            left_key,
            right_key,
            anti,
        },

        LogicalPlan::WithCTE { ctes, main_query } => {
            let optimized_ctes = ctes
                .into_iter()
                .map(|(name, plan)| (name, Box::new(apply_cse_recursive(*plan, config))))
                .collect();

            LogicalPlan::WithCTE {
                ctes: optimized_ctes,
                main_query: Box::new(apply_cse_recursive(*main_query, config)),
            }
        }

        LogicalPlan::Subquery {
            input,
            alias,
            schema,
        } => LogicalPlan::Subquery {
            input: Box::new(apply_cse_recursive(*input, config)),
            alias,
            schema,
        },

        LogicalPlan::Window {
            input,
            window_exprs,
        } => LogicalPlan::Window {
            input: Box::new(apply_cse_recursive(*input, config)),
            window_exprs,
        },

        LogicalPlan::LateralMap {
            input,
            function_expr,
            column_name,
        } => LogicalPlan::LateralMap {
            input: Box::new(apply_cse_recursive(*input, config)),
            function_expr,
            column_name,
        },

        // Leaf nodes - no recursion needed
        LogicalPlan::Scan { .. }
        | LogicalPlan::TableFunction { .. }
        | LogicalPlan::CTEScan { .. }
        | LogicalPlan::Insert { .. }
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
