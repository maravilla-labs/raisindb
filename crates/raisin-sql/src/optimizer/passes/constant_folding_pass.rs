//! Constant Folding Pass Implementation
//!
//! Applies constant folding to all expressions in the logical plan.

use crate::logical_plan::{FilterPredicate, LogicalPlan};

use super::super::constant_fold::fold_constants;
use super::super::Optimizer;

impl Optimizer {
    /// Apply constant folding to all expressions in the plan
    #[allow(clippy::only_used_in_recursion)]
    pub(crate) fn apply_constant_folding(&self, plan: LogicalPlan) -> LogicalPlan {
        match plan {
            LogicalPlan::Scan { .. } => plan, // No expressions to fold in Scan

            LogicalPlan::TableFunction { .. } => plan,

            LogicalPlan::Filter { input, predicate } => {
                // Fold constants in filter predicate
                let folded_conjuncts: Vec<_> = predicate
                    .conjuncts
                    .into_iter()
                    .map(fold_constants)
                    .collect();

                LogicalPlan::Filter {
                    input: Box::new(self.apply_constant_folding(*input)),
                    predicate: FilterPredicate {
                        conjuncts: folded_conjuncts,
                        canonical: predicate.canonical, // Preserve canonical predicates
                    },
                }
            }

            LogicalPlan::Project { input, exprs } => {
                // Fold constants in projection expressions
                let folded_exprs = exprs
                    .into_iter()
                    .map(|proj| {
                        let mut folded_proj = proj;
                        folded_proj.expr = fold_constants(folded_proj.expr);
                        folded_proj
                    })
                    .collect();

                LogicalPlan::Project {
                    input: Box::new(self.apply_constant_folding(*input)),
                    exprs: folded_exprs,
                }
            }

            LogicalPlan::Sort { input, sort_exprs } => {
                // Fold constants in sort expressions
                let folded_sort_exprs = sort_exprs
                    .into_iter()
                    .map(|sort| {
                        let mut folded_sort = sort;
                        folded_sort.expr = fold_constants(folded_sort.expr);
                        folded_sort
                    })
                    .collect();

                LogicalPlan::Sort {
                    input: Box::new(self.apply_constant_folding(*input)),
                    sort_exprs: folded_sort_exprs,
                }
            }

            LogicalPlan::Limit {
                input,
                limit,
                offset,
            } => LogicalPlan::Limit {
                input: Box::new(self.apply_constant_folding(*input)),
                limit,
                offset,
            },

            LogicalPlan::Distinct {
                input,
                distinct_spec,
            } => {
                // Fold constants in DISTINCT ON expressions if present
                let folded_spec = match distinct_spec {
                    crate::logical_plan::operators::DistinctSpec::All => {
                        crate::logical_plan::operators::DistinctSpec::All
                    }
                    crate::logical_plan::operators::DistinctSpec::On(exprs) => {
                        let folded_exprs = exprs.into_iter().map(fold_constants).collect();
                        crate::logical_plan::operators::DistinctSpec::On(folded_exprs)
                    }
                };

                LogicalPlan::Distinct {
                    input: Box::new(self.apply_constant_folding(*input)),
                    distinct_spec: folded_spec,
                }
            }

            LogicalPlan::Aggregate {
                input,
                group_by,
                aggregates,
            } => {
                // Fold constants in group by and aggregate expressions
                let folded_group_by = group_by.into_iter().map(fold_constants).collect();

                let folded_aggregates = aggregates
                    .into_iter()
                    .map(|agg| {
                        let mut folded_agg = agg;
                        folded_agg.args = folded_agg.args.into_iter().map(fold_constants).collect();
                        folded_agg
                    })
                    .collect();

                LogicalPlan::Aggregate {
                    input: Box::new(self.apply_constant_folding(*input)),
                    group_by: folded_group_by,
                    aggregates: folded_aggregates,
                }
            }

            LogicalPlan::Join {
                left,
                right,
                join_type,
                condition,
            } => {
                // Fold constants in join condition if present
                let folded_condition = condition.map(fold_constants);
                LogicalPlan::Join {
                    left: Box::new(self.apply_constant_folding(*left)),
                    right: Box::new(self.apply_constant_folding(*right)),
                    join_type,
                    condition: folded_condition,
                }
            }

            LogicalPlan::SemiJoin {
                left,
                right,
                left_key,
                right_key,
                anti,
            } => {
                // Fold constants in join keys
                LogicalPlan::SemiJoin {
                    left: Box::new(self.apply_constant_folding(*left)),
                    right: Box::new(self.apply_constant_folding(*right)),
                    left_key: fold_constants(left_key),
                    right_key: fold_constants(right_key),
                    anti,
                }
            }

            LogicalPlan::WithCTE { ctes, main_query } => {
                // Apply constant folding to CTEs and main query
                let folded_ctes: Vec<(String, Box<LogicalPlan>)> = ctes
                    .into_iter()
                    .map(|(name, plan)| (name, Box::new(self.apply_constant_folding(*plan))))
                    .collect();

                LogicalPlan::WithCTE {
                    ctes: folded_ctes,
                    main_query: Box::new(self.apply_constant_folding(*main_query)),
                }
            }

            LogicalPlan::CTEScan { .. } => plan,

            LogicalPlan::Subquery {
                input,
                alias,
                schema,
            } => {
                // Apply constant folding to the subquery's input plan
                LogicalPlan::Subquery {
                    input: Box::new(self.apply_constant_folding(*input)),
                    alias,
                    schema,
                }
            }

            LogicalPlan::Window {
                input,
                window_exprs,
            } => {
                // Fold constants in window expressions
                let folded_window_exprs = window_exprs
                    .into_iter()
                    .map(|mut window_expr| {
                        // Fold in partition by
                        window_expr.partition_by = window_expr
                            .partition_by
                            .into_iter()
                            .map(fold_constants)
                            .collect();
                        // Fold in order by
                        window_expr.order_by = window_expr
                            .order_by
                            .into_iter()
                            .map(|(expr, desc)| (fold_constants(expr), desc))
                            .collect();
                        // Fold in window function arguments
                        window_expr.function = match window_expr.function {
                            crate::analyzer::WindowFunction::Sum(expr) => {
                                crate::analyzer::WindowFunction::Sum(Box::new(fold_constants(
                                    *expr,
                                )))
                            }
                            crate::analyzer::WindowFunction::Avg(expr) => {
                                crate::analyzer::WindowFunction::Avg(Box::new(fold_constants(
                                    *expr,
                                )))
                            }
                            crate::analyzer::WindowFunction::Min(expr) => {
                                crate::analyzer::WindowFunction::Min(Box::new(fold_constants(
                                    *expr,
                                )))
                            }
                            crate::analyzer::WindowFunction::Max(expr) => {
                                crate::analyzer::WindowFunction::Max(Box::new(fold_constants(
                                    *expr,
                                )))
                            }
                            other => other,
                        };
                        window_expr
                    })
                    .collect();

                LogicalPlan::Window {
                    input: Box::new(self.apply_constant_folding(*input)),
                    window_exprs: folded_window_exprs,
                }
            }

            LogicalPlan::LateralMap {
                input,
                function_expr,
                column_name,
            } => {
                // Fold constants in the lateral function expression
                LogicalPlan::LateralMap {
                    input: Box::new(self.apply_constant_folding(*input)),
                    function_expr: fold_constants(function_expr),
                    column_name,
                }
            }

            // DML operations and empty plans - no constant folding needed (leaf nodes)
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
}
