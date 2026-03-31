//! Hierarchy Rewriting Pass Implementation
//!
//! Applies hierarchy function rewriting to filter predicates, transforming
//! hierarchy-specific functions into canonical predicates for efficient execution.

use crate::logical_plan::{FilterPredicate, LogicalPlan};

use super::super::hierarchy_rewrite::rewrite_hierarchy_predicates;
use super::super::Optimizer;

impl Optimizer {
    /// Apply hierarchy function rewriting to filter predicates
    ///
    /// This transforms hierarchy-specific functions into canonical predicates
    /// that can be efficiently executed using RocksDB indexes.
    #[allow(clippy::only_used_in_recursion)]
    pub(crate) fn apply_hierarchy_rewriting(&self, plan: LogicalPlan) -> LogicalPlan {
        match plan {
            LogicalPlan::Filter { input, predicate } => {
                // Rewrite each conjunct
                let mut canonical_predicates = Vec::new();

                for conjunct in &predicate.conjuncts {
                    let rewritten = rewrite_hierarchy_predicates(conjunct.clone());
                    canonical_predicates.extend(rewritten);
                }

                // Convert canonical predicates back to typed expressions
                let rewritten_conjuncts: Vec<_> = canonical_predicates
                    .iter()
                    .map(|pred| pred.to_expr())
                    .collect();

                LogicalPlan::Filter {
                    input: Box::new(self.apply_hierarchy_rewriting(*input)),
                    predicate: FilterPredicate {
                        conjuncts: rewritten_conjuncts,
                        canonical: Some(canonical_predicates), // Store canonical form
                    },
                }
            }

            // Recursively apply to other plan nodes
            LogicalPlan::Project { input, exprs } => LogicalPlan::Project {
                input: Box::new(self.apply_hierarchy_rewriting(*input)),
                exprs,
            },

            LogicalPlan::Sort { input, sort_exprs } => LogicalPlan::Sort {
                input: Box::new(self.apply_hierarchy_rewriting(*input)),
                sort_exprs,
            },

            LogicalPlan::Limit {
                input,
                limit,
                offset,
            } => LogicalPlan::Limit {
                input: Box::new(self.apply_hierarchy_rewriting(*input)),
                limit,
                offset,
            },

            LogicalPlan::Distinct {
                input,
                distinct_spec,
            } => LogicalPlan::Distinct {
                input: Box::new(self.apply_hierarchy_rewriting(*input)),
                distinct_spec,
            },

            LogicalPlan::Aggregate {
                input,
                group_by,
                aggregates,
            } => LogicalPlan::Aggregate {
                input: Box::new(self.apply_hierarchy_rewriting(*input)),
                group_by,
                aggregates,
            },

            LogicalPlan::Join {
                left,
                right,
                join_type,
                condition,
            } => LogicalPlan::Join {
                left: Box::new(self.apply_hierarchy_rewriting(*left)),
                right: Box::new(self.apply_hierarchy_rewriting(*right)),
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
                left: Box::new(self.apply_hierarchy_rewriting(*left)),
                right: Box::new(self.apply_hierarchy_rewriting(*right)),
                left_key,
                right_key,
                anti,
            },

            LogicalPlan::WithCTE { ctes, main_query } => {
                let rewritten_ctes: Vec<(String, Box<LogicalPlan>)> = ctes
                    .into_iter()
                    .map(|(name, plan)| (name, Box::new(self.apply_hierarchy_rewriting(*plan))))
                    .collect();

                LogicalPlan::WithCTE {
                    ctes: rewritten_ctes,
                    main_query: Box::new(self.apply_hierarchy_rewriting(*main_query)),
                }
            }

            LogicalPlan::Subquery {
                input,
                alias,
                schema,
            } => {
                // Apply hierarchy rewriting to the subquery's input plan
                LogicalPlan::Subquery {
                    input: Box::new(self.apply_hierarchy_rewriting(*input)),
                    alias,
                    schema,
                }
            }

            LogicalPlan::Window {
                input,
                window_exprs,
            } => LogicalPlan::Window {
                input: Box::new(self.apply_hierarchy_rewriting(*input)),
                window_exprs,
            },

            LogicalPlan::LateralMap {
                input,
                function_expr,
                column_name,
            } => LogicalPlan::LateralMap {
                input: Box::new(self.apply_hierarchy_rewriting(*input)),
                function_expr,
                column_name,
            },

            // Scan, CTE scans, table functions, DML operations, and empty plans don't need rewriting
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
}
