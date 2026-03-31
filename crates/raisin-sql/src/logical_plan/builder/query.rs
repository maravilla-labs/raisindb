//! Main SELECT query plan building.

use super::PlanBuilder;
use crate::analyzer::{AnalyzedDistinct, AnalyzedQuery};
use crate::logical_plan::{
    error::{PlanError, Result},
    operators::{DistinctSpec, FilterPredicate, LogicalPlan, ProjectionExpr, SortExpr, WindowExpr},
};

impl<'a> PlanBuilder<'a> {
    pub(crate) fn build_query(&self, query: &AnalyzedQuery) -> Result<LogicalPlan> {
        // TODO: For now, only handle single-table queries
        // Join support will be added in Phase 2
        if query.from.is_empty() {
            return Err(PlanError::InvalidPlan(
                "FROM clause has no tables".to_string(),
            ));
        }

        // Derive aliases for each projection expression upfront
        // This keeps alias usage consistent across projection, ORDER BY, etc.
        let projection_aliases: Vec<String> = query
            .projection
            .iter()
            .map(|(expr, alias)| {
                alias
                    .clone()
                    .unwrap_or_else(|| Self::derive_column_name(expr))
            })
            .collect();

        // Split WHERE clause predicates by table reference
        let (table_predicates, remaining_predicates) = if let Some(predicate) = &query.selection {
            // Collect all table refs (FROM + JOINs)
            let mut all_table_refs = vec![query.from[0].clone()];
            for join_info in &query.joins {
                all_table_refs.push(join_info.right_table.clone());
            }

            Self::split_predicates_by_table(predicate, &all_table_refs)
        } else {
            (std::collections::HashMap::new(), Vec::new())
        };

        // 1. Start with first table source (table or function)
        // Pass table-specific predicates to the scan
        let first_table_name = query.from[0]
            .alias
            .clone()
            .unwrap_or_else(|| query.from[0].table.clone());
        let first_table_filter = table_predicates
            .get(&first_table_name)
            .and_then(|preds| Self::combine_with_and(preds.clone()));

        let mut plan = self.build_table_source(&query.from[0], query, first_table_filter)?;

        // 1.5. Add joins if present
        for join_info in &query.joins {
            // Check if this is a LATERAL function - emit LateralMap instead of Join
            if let Some(lateral_fn) = &join_info.right_table.lateral_function {
                plan = LogicalPlan::LateralMap {
                    input: Box::new(plan),
                    function_expr: lateral_fn.function_expr.clone(),
                    column_name: lateral_fn.column_name.clone(),
                };
                continue;
            }

            // Build scan for right table with its predicates
            let right_table = &join_info.right_table;
            let right_table_name = right_table
                .alias
                .clone()
                .unwrap_or_else(|| right_table.table.clone());
            let right_table_filter = table_predicates
                .get(&right_table_name)
                .and_then(|preds| Self::combine_with_and(preds.clone()));

            let right_scan = self.build_table_source(right_table, query, right_table_filter)?;

            // Create Join node
            plan = LogicalPlan::Join {
                left: Box::new(plan),
                right: Box::new(right_scan),
                join_type: join_info.join_type.clone(),
                condition: join_info.condition.clone(),
            };
        }

        // 2. Apply Filter (WHERE clause) - only for remaining predicates
        // that couldn't be pushed down to individual scans
        if !remaining_predicates.is_empty() {
            // Extract IN subquery predicates and handle them as SemiJoins
            let (in_subquery_predicates, other_predicates) =
                Self::extract_in_subquery_predicates(remaining_predicates);

            // Apply regular filter predicates first (if any)
            if !other_predicates.is_empty() {
                let remaining_filter = Self::combine_with_and(other_predicates).unwrap();
                plan = LogicalPlan::Filter {
                    input: Box::new(plan),
                    predicate: FilterPredicate::from_expr(remaining_filter),
                };
            }

            // Apply IN subquery predicates as SemiJoins
            for in_subquery in &in_subquery_predicates {
                plan = self.build_semi_join_from_in_subquery(plan, in_subquery)?;
            }
        }

        // 3. Apply Aggregate (GROUP BY) if present
        if !query.group_by.is_empty() || !query.aggregates.is_empty() {
            plan = LogicalPlan::Aggregate {
                input: Box::new(plan),
                group_by: query.group_by.clone(),
                aggregates: query.aggregates.clone(),
            };
        }

        // 4. Check for window functions in projection and handle them
        let has_windows = query
            .projection
            .iter()
            .any(|(expr, _)| Self::contains_window_function(expr));

        if has_windows {
            use crate::analyzer::Expr;

            let mut window_exprs = Vec::new();
            let mut final_projection = Vec::new();

            // Check if there's a preceding GROUP BY for expression rewriting
            let (group_by_exprs, _agg_exprs) = match &plan {
                LogicalPlan::Aggregate {
                    group_by,
                    aggregates,
                    ..
                } => (Some(group_by.clone()), Some(aggregates.clone())),
                _ => (None, None),
            };

            for (idx, (expr, _alias)) in query.projection.iter().enumerate() {
                let derived_alias = projection_aliases[idx].clone();

                // Check if this is a top-level window function
                if let Expr::Window {
                    function,
                    partition_by,
                    order_by,
                    frame,
                } = &expr.expr
                {
                    // Rewrite PARTITION BY and ORDER BY expressions if they match GROUP BY
                    let (rewritten_partition_by, rewritten_order_by) =
                        if let Some(ref group_by) = group_by_exprs {
                            let new_partition = Self::rewrite_groupby_refs(partition_by, group_by);
                            let new_order: Vec<_> = order_by
                                .iter()
                                .map(|(order_expr, is_desc)| {
                                    // Check if this order expression matches a GROUP BY expression
                                    for group_expr in group_by {
                                        if Self::exprs_match(order_expr, group_expr) {
                                            let canonical_name =
                                                Self::generate_groupby_column_name(group_expr);
                                            return (
                                                crate::analyzer::TypedExpr::new(
                                                    Expr::Column {
                                                        table: "".to_string(),
                                                        column: canonical_name,
                                                    },
                                                    order_expr.data_type.clone(),
                                                ),
                                                *is_desc,
                                            );
                                        }
                                    }
                                    // No match - keep as-is
                                    (order_expr.clone(), *is_desc)
                                })
                                .collect();
                            (new_partition, new_order)
                        } else {
                            (partition_by.clone(), order_by.clone())
                        };

                    // This is a window function - add to window expressions with rewritten specs
                    window_exprs.push(WindowExpr {
                        function: function.clone(),
                        partition_by: rewritten_partition_by,
                        order_by: rewritten_order_by,
                        frame: frame.clone(),
                        alias: derived_alias.clone(),
                        return_type: expr.data_type.clone(),
                    });

                    // Final projection references this computed column
                    final_projection.push(ProjectionExpr {
                        expr: crate::analyzer::TypedExpr::new(
                            Expr::Column {
                                table: "".to_string(),
                                column: derived_alias.clone(),
                            },
                            expr.data_type.clone(),
                        ),
                        alias: derived_alias,
                    });
                } else {
                    // Expression may contain embedded window functions
                    // Extract them and replace with column references
                    let (mut rewritten_expr, mut extracted_windows) =
                        Self::extract_window_functions(expr);

                    // Add extracted window functions to the list
                    window_exprs.append(&mut extracted_windows);

                    // Check if this expression matches a GROUP BY expression
                    if let Some(ref group_by) = group_by_exprs {
                        for group_expr in group_by {
                            if Self::exprs_match(&rewritten_expr, group_expr) {
                                // Match found! Replace with column reference
                                let canonical_name = Self::generate_groupby_column_name(group_expr);
                                tracing::debug!(
                                    "Rewriting non-window expression '{}' to reference GROUP BY column '{}'",
                                    derived_alias,
                                    canonical_name
                                );

                                rewritten_expr = crate::analyzer::TypedExpr::new(
                                    Expr::Column {
                                        table: "".to_string(),
                                        column: canonical_name,
                                    },
                                    rewritten_expr.data_type.clone(),
                                );
                                break;
                            }
                        }
                    }

                    // Add the rewritten expression to final projection
                    final_projection.push(ProjectionExpr {
                        expr: rewritten_expr,
                        alias: derived_alias,
                    });
                }
            }

            if !window_exprs.is_empty() {
                plan = LogicalPlan::Window {
                    input: Box::new(plan),
                    window_exprs,
                };
            }

            plan = LogicalPlan::Project {
                input: Box::new(plan),
                exprs: final_projection,
            };
        } else {
            // No window functions - standard projection
            // Check if projection comes after an aggregate - if so, rewrite expressions
            let (group_by_exprs, _agg_exprs) = match &plan {
                LogicalPlan::Aggregate {
                    group_by,
                    aggregates,
                    ..
                } => (Some(group_by.clone()), Some(aggregates.clone())),
                _ => (None, None),
            };

            let projection_exprs: Vec<ProjectionExpr> = query
                .projection
                .iter()
                .enumerate()
                .map(|(idx, (expr, _alias))| {
                    let derived_alias = projection_aliases[idx].clone();

                    // If this projection comes after GROUP BY, check if expression matches a GROUP BY expr
                    if let Some(ref group_by) = group_by_exprs {
                        for group_expr in group_by {
                            if Self::exprs_match(expr, group_expr) {
                                // Match found! Replace with column reference
                                let canonical_name = Self::generate_groupby_column_name(group_expr);
                                tracing::debug!(
                                    "Rewriting SELECT expression '{}' to reference GROUP BY column '{}'",
                                    derived_alias,
                                    canonical_name
                                );

                                return ProjectionExpr {
                                    expr: crate::analyzer::TypedExpr::new(
                                        crate::analyzer::Expr::Column {
                                            table: "".to_string(),
                                            column: canonical_name,
                                        },
                                        expr.data_type.clone(),
                                    ),
                                    alias: derived_alias,
                                };
                            }
                        }
                    }

                    // No match found or no GROUP BY - keep expression as-is
                    ProjectionExpr {
                        expr: expr.clone(),
                        alias: derived_alias,
                    }
                })
                .collect();

            plan = LogicalPlan::Project {
                input: Box::new(plan),
                exprs: projection_exprs,
            };
        }

        // 4.5. Apply DISTINCT ALL if present (before Sort)
        // DISTINCT ON will be applied after Sort for "first row" semantics
        let mut deferred_distinct_on = None;
        if let Some(ref distinct) = query.distinct {
            match distinct {
                AnalyzedDistinct::All => {
                    // Basic DISTINCT goes after Project, before Sort
                    plan = LogicalPlan::Distinct {
                        input: Box::new(plan),
                        distinct_spec: DistinctSpec::All,
                    };
                }
                AnalyzedDistinct::On(ref exprs) => {
                    // DISTINCT ON will be applied after Sort - defer for now
                    deferred_distinct_on = Some(exprs.clone());
                }
            }
        }

        // 5. Apply Sort (ORDER BY clause)
        if !query.order_by.is_empty() {
            let sort_exprs: Vec<SortExpr> = query
                .order_by
                .iter()
                .map(|order_spec| SortExpr {
                    expr: Self::rewrite_order_by_expr(&order_spec.expr, query, &projection_aliases),
                    ascending: !order_spec.descending,
                    nulls_first: order_spec.nulls_first(),
                })
                .collect();

            plan = LogicalPlan::Sort {
                input: Box::new(plan),
                sort_exprs,
            };
        }

        // 5.5. Apply DISTINCT ON if present (after Sort)
        // DISTINCT ON requires sorted input for "first row per group" semantics
        if let Some(exprs) = deferred_distinct_on {
            plan = LogicalPlan::Distinct {
                input: Box::new(plan),
                distinct_spec: DistinctSpec::On(exprs),
            };
        }

        // 6. Apply Limit/Offset
        if query.limit.is_some() || query.offset.is_some() {
            plan = LogicalPlan::Limit {
                input: Box::new(plan),
                limit: query.limit.unwrap_or(usize::MAX),
                offset: query.offset.unwrap_or(0),
            };
        }

        // 7. Wrap in WithCTE if there are CTEs
        if !query.ctes.is_empty() {
            // Build logical plans for each CTE
            let mut cte_plans = Vec::new();
            for (cte_name, cte_query) in &query.ctes {
                let cte_plan = self.build_query(cte_query)?;
                cte_plans.push((cte_name.clone(), Box::new(cte_plan)));
            }

            plan = LogicalPlan::WithCTE {
                ctes: cte_plans,
                main_query: Box::new(plan),
            };
        }

        Ok(plan)
    }
}
