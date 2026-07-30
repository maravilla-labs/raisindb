//! Core planning dispatch
//!
//! Contains the main `plan_with_context` method that converts each
//! `LogicalPlan` variant into the corresponding `PhysicalPlan` node.
//!
//! # Sub-modules
//!
//! - `scan` - Scan and TableFunction dispatch
//! - `limit` - Limit planning with pushdown and vector k-NN
//! - `aggregate` - Aggregate planning with COUNT(*) optimizations
//! - `dml` - DML operation dispatch (Insert, Update, Delete, etc.)
//! - `spatial_knn` - Spatial k-NN (`ORDER BY ST_DISTANCE ... LIMIT k`) detection
//! - `vector_knn` - Vector k-NN pattern detection within limit planning

mod aggregate;
mod dml;
mod limit;
mod scan;
mod spatial_knn;
mod vector_knn;

use super::{Error, LogicalPlan, PhysicalPlan, PhysicalPlanner, PlanContext, SortExpr};
use std::sync::Arc;

impl PhysicalPlanner {
    /// Convert a logical plan to a physical plan with context from parent operators
    pub(super) fn plan_with_context(
        &self,
        logical: &LogicalPlan,
        context: &PlanContext,
    ) -> Result<PhysicalPlan, Error> {
        // Try DML operations first (thin 1-to-1 mappings)
        if let Some(result) = self.try_plan_dml(logical) {
            return result;
        }

        match logical {
            LogicalPlan::Scan {
                table,
                alias,
                schema,
                workspace,
                max_revision: _,
                branch_override,
                locales: _,
                filter,
                projection,
            } => self.plan_scan(
                table,
                alias,
                schema.clone(),
                workspace,
                branch_override,
                filter,
                projection,
                context,
            ),

            LogicalPlan::TableFunction {
                name,
                alias,
                args,
                schema,
                workspace,
                branch_override,
                max_revision,
                locales: _,
            } => self.plan_table_function(
                name,
                alias,
                args,
                schema,
                workspace,
                branch_override,
                *max_revision,
            ),

            LogicalPlan::Filter { input, predicate } => {
                // Check if input is a Scan - if so, we can push predicates down for scan selection
                if let LogicalPlan::Scan {
                    table,
                    alias,
                    schema,
                    workspace,
                    max_revision: _,
                    branch_override,
                    locales: _,
                    filter: scan_filter,
                    projection,
                } = input.as_ref()
                {
                    // Combine Scan's filter (if any) with Filter's predicates
                    let combined_filter = if let Some(scan_filter_expr) = scan_filter {
                        // Both Scan and Filter have predicates - combine with AND
                        let mut all_predicates = vec![scan_filter_expr.clone()];
                        all_predicates.extend(predicate.conjuncts.clone());

                        // Combine into single expression
                        self.combine_predicates(&all_predicates)
                    } else {
                        // Only Filter has predicates
                        self.combine_predicates(&predicate.conjuncts)
                    };

                    let workspace_name = workspace
                        .clone()
                        .unwrap_or_else(|| self.default_workspace.to_string());
                    let effective_branch = branch_override
                        .clone()
                        .unwrap_or_else(|| self.default_branch.to_string());

                    // Plan scan with combined filter for intelligent scan selection
                    return self.plan_scan_with_filter(
                        table,
                        alias,
                        schema.clone(),
                        &workspace_name,
                        &effective_branch,
                        &combined_filter,
                        projection.clone(),
                        context, // Pass through parent context
                    );
                }

                // Input is not a Scan - plan it normally and wrap with Filter
                let input_plan = self.plan_with_context(input, context)?;
                Ok(PhysicalPlan::Filter {
                    input: Box::new(input_plan),
                    predicates: predicate.conjuncts.clone(),
                })
            }

            LogicalPlan::Project { input, exprs } => {
                let input_plan = self.plan_with_context(input, context)?;
                Ok(PhysicalPlan::Project {
                    input: Box::new(input_plan),
                    exprs: exprs.clone(),
                })
            }

            LogicalPlan::Sort { input, sort_exprs } => {
                // Extract sort column and direction to pass to child operators
                let mut new_context = context.clone();
                if let Some(first_sort) = sort_exprs.first() {
                    let is_asc = first_sort.ascending;
                    // Always record the verbatim expression: a scan needs to know
                    // an ORDER BY exists even when its key is not a column name
                    // (e.g. ORDER BY ST_DISTANCE(...)), or it will happily bound
                    // its output in its OWN order and return the wrong rows.
                    new_context = new_context.with_order_by_expr(first_sort.expr.clone(), is_asc);
                    if let Some(column_name) = Self::extract_column_name(&first_sort.expr) {
                        new_context = new_context.with_order_by(column_name, is_asc);
                        tracing::debug!(
                            "Propagating ORDER BY {:?} to child operators",
                            new_context.order_by
                        );
                    }
                }

                let input_plan = self.plan_with_context(input, &new_context)?;

                // Sort elision: if the chosen scan already emits exactly the
                // requested order, sorting again is pure cost (and, with a
                // LIMIT, forces a full materialization the scan could have
                // avoided).
                //
                // Deliberately narrow — a single sort expression on an editorial
                // ordering column, over a scan that has claimed that ordering.
                // This is not a general sort-elision framework.
                if Self::sort_is_satisfied_by_scan(sort_exprs, &input_plan) {
                    tracing::info!(
                        "⚡ Eliding Sort: scan already emits editorial order for {:?}",
                        new_context.order_by
                    );
                    return Ok(input_plan);
                }

                Ok(PhysicalPlan::Sort {
                    input: Box::new(input_plan),
                    sort_exprs: sort_exprs.clone(),
                })
            }

            LogicalPlan::Limit {
                input,
                limit,
                offset,
            } => self.plan_limit(input, *limit, *offset, context),

            LogicalPlan::Aggregate {
                input,
                group_by,
                aggregates,
            } => self.plan_aggregate(input, group_by, aggregates, context),

            LogicalPlan::Join {
                left,
                right,
                join_type,
                condition,
            } => {
                // Plan both inputs (pass through context)
                let left_plan = self.plan_with_context(left, context)?;
                let right_plan = self.plan_with_context(right, context)?;

                // Which table qualifiers does each side produce? Used to assign
                // join-condition operands (including expression keys like
                // properties->>'x') to the correct side.
                let mut left_qualifiers = std::collections::HashSet::new();
                let mut right_qualifiers = std::collections::HashSet::new();
                Self::collect_plan_qualifiers(left, &mut left_qualifiers);
                Self::collect_plan_qualifiers(right, &mut right_qualifiers);

                // Analyze condition to choose between HashJoin, IndexLookupJoin, and NestedLoopJoin
                if let Some((left_keys, right_keys)) =
                    Self::extract_equality_join_keys(condition, &left_qualifiers, &right_qualifiers)
                {
                    // Check if we can use IndexLookupJoin (O(n) vs O(n+m) for HashJoin)
                    if matches!(
                        join_type,
                        raisin_sql::analyzer::JoinType::Inner
                            | raisin_sql::analyzer::JoinType::Left
                    ) {
                        // Try right side as the indexed lookup (left as outer)
                        if let Some(index_lookup_plan) = self.try_create_index_lookup_join(
                            &left_plan,
                            &right_plan,
                            join_type.clone(),
                            &left_keys,
                            &right_keys,
                        ) {
                            tracing::debug!(
                                "Using IndexLookupJoin (left outer, right index lookup)"
                            );
                            return Ok(index_lookup_plan);
                        }

                        // Try left side as the indexed lookup (right as outer)
                        if let Some(index_lookup_plan) = self.try_create_index_lookup_join(
                            &right_plan,
                            &left_plan,
                            join_type.clone(),
                            &right_keys,
                            &left_keys,
                        ) {
                            tracing::debug!(
                                "Using IndexLookupJoin (right outer, left index lookup)"
                            );
                            return Ok(index_lookup_plan);
                        }
                    }

                    // Fall back to HashJoin for equality joins
                    tracing::debug!(
                        "Using HashJoin for equality join with {} key(s)",
                        left_keys.len()
                    );
                    Ok(PhysicalPlan::HashJoin {
                        left: Box::new(left_plan),
                        right: Box::new(right_plan),
                        join_type: join_type.clone(),
                        left_keys,
                        right_keys,
                    })
                } else {
                    // Fall back to NestedLoopJoin for non-equality or complex conditions
                    tracing::debug!("Using NestedLoopJoin (non-equality or complex condition)");
                    Ok(PhysicalPlan::NestedLoopJoin {
                        left: Box::new(left_plan),
                        right: Box::new(right_plan),
                        join_type: join_type.clone(),
                        condition: condition.clone(),
                    })
                }
            }

            LogicalPlan::WithCTE { ctes, main_query } => {
                // Plan each CTE and the main query
                let mut planned_ctes = Vec::new();
                for (name, cte_plan) in ctes {
                    // CTEs are evaluated independently, so use empty context
                    planned_ctes.push((
                        name.clone(),
                        Box::new(self.plan_with_context(cte_plan, &PlanContext::empty())?),
                    ));
                }

                let planned_main = self.plan_with_context(main_query, context)?;

                Ok(PhysicalPlan::WithCTE {
                    ctes: planned_ctes,
                    main_query: Box::new(planned_main),
                })
            }

            LogicalPlan::CTEScan {
                cte_name,
                schema,
                alias: _,
            } => Ok(PhysicalPlan::CTEScan {
                cte_name: cte_name.clone(),
                schema: Arc::clone(schema),
            }),

            LogicalPlan::Subquery {
                input,
                alias,
                schema,
            } => {
                // Subquery - materialize inline and return a scan over the results
                let subquery_plan = self.plan_with_context(input, &PlanContext::empty())?;

                let cte_scan = PhysicalPlan::CTEScan {
                    cte_name: alias.clone(),
                    schema: Arc::clone(schema),
                };

                Ok(PhysicalPlan::WithCTE {
                    ctes: vec![(alias.clone(), Box::new(subquery_plan))],
                    main_query: Box::new(cte_scan),
                })
            }

            LogicalPlan::Window {
                input,
                window_exprs,
            } => {
                let input_plan = self.plan_with_context(input, context)?;
                Ok(PhysicalPlan::Window {
                    input: Box::new(input_plan),
                    window_exprs: window_exprs.clone(),
                })
            }

            LogicalPlan::Distinct {
                input,
                distinct_spec,
            } => {
                let input_plan = self.plan_with_context(input, context)?;
                let on_columns = match distinct_spec {
                    raisin_sql::logical_plan::DistinctSpec::All => vec![],
                    raisin_sql::logical_plan::DistinctSpec::On(exprs) => exprs
                        .iter()
                        .filter_map(|e| {
                            if let raisin_sql::analyzer::Expr::Column { column, .. } = &e.expr {
                                Some(column.clone())
                            } else {
                                None
                            }
                        })
                        .collect(),
                };
                Ok(PhysicalPlan::Distinct {
                    input: Box::new(input_plan),
                    on_columns,
                })
            }

            LogicalPlan::LateralMap {
                input,
                function_expr,
                column_name,
            } => {
                let input_plan = self.plan_with_context(input, context)?;
                Ok(PhysicalPlan::LateralMap {
                    input: Box::new(input_plan),
                    function_expr: function_expr.clone(),
                    column_name: column_name.clone(),
                })
            }

            // Semi-join for IN subquery support
            LogicalPlan::SemiJoin {
                left,
                right,
                left_key,
                right_key,
                anti,
            } => {
                let physical_left = self.plan(left)?;
                let physical_right = self.plan(right)?;

                Ok(PhysicalPlan::HashSemiJoin {
                    left: Box::new(physical_left),
                    right: Box::new(physical_right),
                    left_key: left_key.clone(),
                    right_key: right_key.clone(),
                    anti: *anti,
                })
            }

            // Empty plan - used for DDL statements that bypass logical planning
            LogicalPlan::Empty => Ok(PhysicalPlan::Empty),

            // DML variants are handled by try_plan_dml above; reaching here
            // means a new DML variant was added without updating that function.
            LogicalPlan::Insert { .. }
            | LogicalPlan::Update { .. }
            | LogicalPlan::Delete { .. }
            | LogicalPlan::Order { .. }
            | LogicalPlan::Move { .. }
            | LogicalPlan::Copy { .. }
            | LogicalPlan::Translate { .. }
            | LogicalPlan::Relate { .. }
            | LogicalPlan::Unrelate { .. } => {
                unreachable!("DML variants should have been handled by try_plan_dml")
            }
        }
    }

    /// True when `input` already emits rows in exactly the order `sort_exprs`
    /// asks for, so the `Sort` above it can be dropped.
    ///
    /// Intentionally narrow: one sort expression, on an editorial ordering
    /// column, over a scan that set `claims_editorial_order`. The scan sets that
    /// flag only when its own iteration order matches the requested direction and
    /// nothing below it can reorder rows, so this check is a lookup rather than a
    /// re-derivation.
    ///
    /// Projections pass through (they preserve row order); anything else does
    /// not, so a Filter or Limit between the Sort and the scan blocks elision.
    fn sort_is_satisfied_by_scan(sort_exprs: &[SortExpr], input: &PhysicalPlan) -> bool {
        if sort_exprs.len() != 1 {
            return false;
        }
        let sort = &sort_exprs[0];

        // Ascending distance order, spelled either as the emitted `__distance`
        // column or as the ST_DISTANCE call itself. Descending is NOT elidable:
        // the index walks nearest-first and has no cheap reverse.
        let by_distance_column = matches!(
            Self::extract_column_name(&sort.expr).as_deref(),
            Some("__distance")
        );
        let by_distance_expr =
            raisin_sql::optimizer::hierarchy_rewrite::extract_distance_order(&sort.expr)
                .filter(|order| order.exact);
        let wants_distance_order =
            sort.ascending && (by_distance_column || by_distance_expr.is_some());

        let editorial_column = Self::extract_column_name(&sort.expr)
            .filter(|c| matches!(c.as_str(), "__order" | "__tree_order"));

        if editorial_column.is_none() && !wants_distance_order {
            return false;
        }

        let mut node = input;
        loop {
            match node {
                PhysicalPlan::Project { input, .. } => node = input,
                PhysicalPlan::PrefixScan {
                    claims_editorial_order,
                    ..
                } => return editorial_column.is_some() && *claims_editorial_order,
                PhysicalPlan::SpatialDistanceScan {
                    property_name,
                    center_lon,
                    center_lat,
                    claims_distance_order,
                    ..
                }
                | PhysicalPlan::SpatialKnnScan {
                    property_name,
                    center_lon,
                    center_lat,
                    claims_distance_order,
                    ..
                } => {
                    if !wants_distance_order || !*claims_distance_order {
                        return false;
                    }
                    // An explicit ST_DISTANCE key must be measuring the SAME
                    // property from the SAME centre as the scan, or the scan's
                    // order is not the requested order at all.
                    return match &by_distance_expr {
                        Some(order) => {
                            order.property_name == *property_name
                                && order.center_lon == *center_lon
                                && order.center_lat == *center_lat
                        }
                        // `__distance` is by definition the column this scan
                        // emits, so it always matches.
                        None => true,
                    };
                }
                _ => return false,
            }
        }
    }
}
