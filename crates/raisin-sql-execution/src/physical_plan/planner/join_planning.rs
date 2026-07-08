//! Join optimization
//!
//! Implements IndexLookupJoin for efficient joins when the inner side
//! is a workspace table with id/path lookups.

use super::super::operators::{IndexLookupParams, IndexLookupType};
use super::{Expr, PhysicalPlan, PhysicalPlanner, TypedExpr};
use raisin_sql::analyzer::BinaryOperator;

impl PhysicalPlanner {
    /// Try to create an IndexLookupJoin plan
    ///
    /// This is optimal when:
    /// 1. The inner side is a TableScan on a workspace table
    /// 2. The join key on the inner side is the `id` column (primary key)
    /// 3. The outer side is likely small (CTE, TableFunction, subquery)
    ///
    /// Returns Some(PhysicalPlan) if IndexLookupJoin can be used, None otherwise.
    pub(super) fn try_create_index_lookup_join(
        &self,
        outer_plan: &PhysicalPlan,
        inner_plan: &PhysicalPlan,
        join_type: raisin_sql::analyzer::JoinType,
        outer_keys: &[TypedExpr],
        inner_keys: &[TypedExpr],
    ) -> Option<PhysicalPlan> {
        // We need exactly one join key for IndexLookupJoin
        if outer_keys.len() != 1 || inner_keys.len() != 1 {
            return None;
        }

        // Check if outer is a "small" input (CTE scan, table function, or already filtered)
        let is_outer_small = matches!(
            outer_plan,
            PhysicalPlan::CTEScan { .. }
                | PhysicalPlan::TableFunction { .. }
                // Also consider filtered inputs as potentially small
                | PhysicalPlan::Filter { .. }
                // And subqueries wrapped in WithCTE
                | PhysicalPlan::WithCTE { .. }
        );

        if !is_outer_small {
            return None;
        }

        // Extract inner table scan info
        let (tenant_id, repo_id, branch, workspace, table, alias, projection) = match inner_plan {
            PhysicalPlan::TableScan {
                tenant_id,
                repo_id,
                branch,
                workspace,
                table,
                alias,
                projection,
                ..
            } => (
                tenant_id.clone(),
                repo_id.clone(),
                branch.clone(),
                workspace.clone(),
                table.clone(),
                alias.clone(),
                projection.clone(),
            ),
            _ => return None,
        };

        // Check if inner join key is an `id` or `path` column
        let (lookup_type, outer_key_column) = match &inner_keys[0].expr {
            Expr::Column { table: _, column } => {
                let col_lower = column.to_lowercase();
                if col_lower == "id" {
                    // Check what column from outer provides the id
                    let outer_col = Self::extract_column_name(&outer_keys[0])?;
                    (IndexLookupType::ById, outer_col)
                } else if col_lower == "path" {
                    let outer_col = Self::extract_column_name(&outer_keys[0])?;
                    (IndexLookupType::ByPath, outer_col)
                } else {
                    return None;
                }
            }
            _ => return None,
        };

        tracing::info!(
            "IndexLookupJoin optimization: outer.{} -> {} lookup on {}",
            outer_key_column,
            match lookup_type {
                IndexLookupType::ById => "id",
                IndexLookupType::ByPath => "path",
            },
            table
        );

        Some(PhysicalPlan::IndexLookupJoin {
            outer: Box::new(outer_plan.clone()),
            join_type,
            outer_key_column,
            inner_lookup: IndexLookupParams {
                lookup_type,
                tenant_id,
                repo_id,
                branch,
                workspace,
                table,
                alias,
                projection,
            },
        })
    }

    /// Collect the table qualifiers (names/aliases) a logical plan's output rows
    /// are qualified with. Used to assign join-condition operands to a side.
    pub(super) fn collect_plan_qualifiers(
        plan: &raisin_sql::logical_plan::LogicalPlan,
        out: &mut std::collections::HashSet<String>,
    ) {
        use raisin_sql::logical_plan::LogicalPlan;
        match plan {
            LogicalPlan::Scan { table, alias, .. } => {
                out.insert(alias.clone().unwrap_or_else(|| table.clone()));
            }
            LogicalPlan::TableFunction { name, alias, .. } => {
                out.insert(alias.clone().unwrap_or_else(|| name.clone()));
            }
            LogicalPlan::CTEScan {
                cte_name, alias, ..
            } => {
                out.insert(alias.clone().unwrap_or_else(|| cte_name.clone()));
                // Columns of a CTE may also be qualified by the CTE's name.
                out.insert(cte_name.clone());
            }
            LogicalPlan::Subquery { alias, .. } => {
                out.insert(alias.clone());
            }
            LogicalPlan::Filter { input, .. }
            | LogicalPlan::Project { input, .. }
            | LogicalPlan::Sort { input, .. }
            | LogicalPlan::Limit { input, .. }
            | LogicalPlan::Distinct { input, .. }
            | LogicalPlan::Aggregate { input, .. }
            | LogicalPlan::Window { input, .. }
            | LogicalPlan::LateralMap { input, .. } => {
                Self::collect_plan_qualifiers(input, out);
            }
            LogicalPlan::Join { left, right, .. } => {
                Self::collect_plan_qualifiers(left, out);
                Self::collect_plan_qualifiers(right, out);
            }
            LogicalPlan::SemiJoin { left, .. } => {
                // A semi-join only outputs the left side's columns.
                Self::collect_plan_qualifiers(left, out);
            }
            LogicalPlan::WithCTE { main_query, .. } => {
                Self::collect_plan_qualifiers(main_query, out);
            }
            // DML / leaf nodes produce no join-relevant qualifiers.
            _ => {}
        }
    }

    /// Extract equality join keys from a join condition
    ///
    /// Analyzes the join condition to determine if it consists of one or more equality
    /// comparisons where each operand references exactly one side of the join. Returns
    /// the left- and right-side key expressions, side-corrected (an `ON b.x = a.y`
    /// condition yields `a.y` in `left_keys` when `a` is the join's left input).
    ///
    /// # Supported Patterns
    ///
    /// - Simple equality: `a.id = b.id`
    /// - Expression keys: `a.properties->>'ref_id' = b.id`, `CAST(a.x AS text) = b.y`
    /// - Multiple equalities (AND): `a.id = b.id AND a.name = b.name`
    ///
    /// `left_qualifiers` / `right_qualifiers` are the table names/aliases produced by
    /// each join input, used to assign each operand to a side.
    ///
    /// # Returns
    ///
    /// `Some((left_keys, right_keys))` if the condition is a valid equality join,
    /// `None` otherwise (OR conditions, non-equality comparisons, operands mixing
    /// both sides, or operands referencing no columns).
    pub(super) fn extract_equality_join_keys(
        condition: &Option<TypedExpr>,
        left_qualifiers: &std::collections::HashSet<String>,
        right_qualifiers: &std::collections::HashSet<String>,
    ) -> Option<(Vec<TypedExpr>, Vec<TypedExpr>)> {
        let condition = condition.as_ref()?;

        let mut left_keys = Vec::new();
        let mut right_keys = Vec::new();

        Self::collect_equality_keys(
            &condition.expr,
            left_qualifiers,
            right_qualifiers,
            &mut left_keys,
            &mut right_keys,
        )?;

        if left_keys.is_empty() {
            return None;
        }

        Some((left_keys, right_keys))
    }

    /// Recursively collect equality join keys from an expression
    ///
    /// Handles:
    /// - BinaryOp::Eq: each operand must reference columns from exactly one side
    /// - BinaryOp::And: Recursively process both sides
    /// - Other operators: Return None (not supported for HashJoin)
    fn collect_equality_keys(
        expr: &Expr,
        left_qualifiers: &std::collections::HashSet<String>,
        right_qualifiers: &std::collections::HashSet<String>,
        left_keys: &mut Vec<TypedExpr>,
        right_keys: &mut Vec<TypedExpr>,
    ) -> Option<()> {
        match expr {
            Expr::BinaryOp { left, op, right } => match op {
                BinaryOperator::Eq => {
                    // Which side of the join does each operand belong to?
                    let lhs_side = Self::expr_side(left, left_qualifiers, right_qualifiers)?;
                    let rhs_side = Self::expr_side(right, left_qualifiers, right_qualifiers)?;

                    match (lhs_side, rhs_side) {
                        (JoinSide::Left, JoinSide::Right) => {
                            left_keys.push(left.as_ref().clone());
                            right_keys.push(right.as_ref().clone());
                            Some(())
                        }
                        (JoinSide::Right, JoinSide::Left) => {
                            left_keys.push(right.as_ref().clone());
                            right_keys.push(left.as_ref().clone());
                            Some(())
                        }
                        // Same-side equality (a.x = a.y) or unknown — not a join key
                        _ => None,
                    }
                }
                BinaryOperator::And => {
                    // For AND, we can collect keys from both sides
                    Self::collect_equality_keys(
                        &left.expr,
                        left_qualifiers,
                        right_qualifiers,
                        left_keys,
                        right_keys,
                    )?;
                    Self::collect_equality_keys(
                        &right.expr,
                        left_qualifiers,
                        right_qualifiers,
                        left_keys,
                        right_keys,
                    )?;
                    Some(())
                }
                _ => {
                    // Other operators (OR, <, >, etc.) not supported for HashJoin
                    None
                }
            },
            _ => {
                // Other expression types not supported
                None
            }
        }
    }

    /// Determine which join side an operand belongs to: it must reference at
    /// least one column and all its column qualifiers must resolve to exactly
    /// one side. Subquery/window expressions are rejected.
    fn expr_side(
        expr: &TypedExpr,
        left_qualifiers: &std::collections::HashSet<String>,
        right_qualifiers: &std::collections::HashSet<String>,
    ) -> Option<JoinSide> {
        let mut qualifiers = std::collections::HashSet::new();
        if !Self::collect_expr_qualifiers(expr, &mut qualifiers) {
            return None;
        }
        if qualifiers.is_empty() {
            // A constant operand is a filter, not a join key.
            return None;
        }
        let all_left = qualifiers.iter().all(|q| left_qualifiers.contains(q));
        let all_right = qualifiers.iter().all(|q| right_qualifiers.contains(q));
        match (all_left, all_right) {
            // Ambiguous (qualifier visible on both sides) or mixed — reject.
            (true, true) | (false, false) => None,
            (true, false) => Some(JoinSide::Left),
            (false, true) => Some(JoinSide::Right),
        }
    }

    /// Collect the table qualifiers referenced by an expression. Returns false
    /// when the expression contains a construct that cannot serve as a hash key
    /// (subqueries, window functions).
    fn collect_expr_qualifiers(
        expr: &TypedExpr,
        out: &mut std::collections::HashSet<String>,
    ) -> bool {
        match &expr.expr {
            Expr::Column { table, .. } => {
                out.insert(table.clone());
                true
            }
            Expr::Literal(_) => true,
            Expr::BinaryOp { left, right, .. } => {
                Self::collect_expr_qualifiers(left, out)
                    && Self::collect_expr_qualifiers(right, out)
            }
            Expr::UnaryOp { expr, .. } | Expr::Cast { expr, .. } => {
                Self::collect_expr_qualifiers(expr, out)
            }
            Expr::IsNull { expr } | Expr::IsNotNull { expr } => {
                Self::collect_expr_qualifiers(expr, out)
            }
            Expr::Between { expr, low, high } => {
                Self::collect_expr_qualifiers(expr, out)
                    && Self::collect_expr_qualifiers(low, out)
                    && Self::collect_expr_qualifiers(high, out)
            }
            Expr::InList { expr, list, .. } => {
                Self::collect_expr_qualifiers(expr, out)
                    && list.iter().all(|e| Self::collect_expr_qualifiers(e, out))
            }
            Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
                Self::collect_expr_qualifiers(expr, out)
                    && Self::collect_expr_qualifiers(pattern, out)
            }
            Expr::JsonExtract { object, key }
            | Expr::JsonExtractText { object, key }
            | Expr::JsonKeyExists { object, key }
            | Expr::JsonRemove { object, key } => {
                Self::collect_expr_qualifiers(object, out)
                    && Self::collect_expr_qualifiers(key, out)
            }
            Expr::JsonContains { object, pattern } => {
                Self::collect_expr_qualifiers(object, out)
                    && Self::collect_expr_qualifiers(pattern, out)
            }
            Expr::JsonAnyKeyExists { object, keys } | Expr::JsonAllKeyExists { object, keys } => {
                Self::collect_expr_qualifiers(object, out)
                    && Self::collect_expr_qualifiers(keys, out)
            }
            Expr::JsonExtractPath { object, path }
            | Expr::JsonExtractPathText { object, path }
            | Expr::JsonRemoveAtPath { object, path }
            | Expr::JsonPathMatch { object, path }
            | Expr::JsonPathExists { object, path } => {
                Self::collect_expr_qualifiers(object, out)
                    && Self::collect_expr_qualifiers(path, out)
            }
            Expr::Function { args, filter, .. } => {
                args.iter().all(|a| Self::collect_expr_qualifiers(a, out))
                    && filter
                        .as_ref()
                        .map(|f| Self::collect_expr_qualifiers(f, out))
                        .unwrap_or(true)
            }
            Expr::Case {
                conditions,
                else_expr,
            } => {
                conditions.iter().all(|(c, r)| {
                    Self::collect_expr_qualifiers(c, out) && Self::collect_expr_qualifiers(r, out)
                }) && else_expr
                    .as_ref()
                    .map(|e| Self::collect_expr_qualifiers(e, out))
                    .unwrap_or(true)
            }
            // Subqueries and window functions cannot be hash-join keys.
            Expr::InSubquery { .. } | Expr::Window { .. } => false,
        }
    }
}

/// Which input of a join an expression's columns belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JoinSide {
    Left,
    Right,
}
