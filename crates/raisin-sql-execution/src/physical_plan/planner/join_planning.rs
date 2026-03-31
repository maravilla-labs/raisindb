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

    /// Extract equality join keys from a join condition
    ///
    /// Analyzes the join condition to determine if it consists of one or more equality
    /// comparisons between columns from the left and right sides. Returns the left and
    /// right join keys if successful.
    ///
    /// # Supported Patterns
    ///
    /// - Simple equality: `a.id = b.id`
    /// - Multiple equalities (AND): `a.id = b.id AND a.name = b.name`
    ///
    /// # Returns
    ///
    /// `Some((left_keys, right_keys))` if the condition is a valid equality join,
    /// `None` otherwise (including OR conditions, non-equality comparisons, etc.)
    pub(super) fn extract_equality_join_keys(
        condition: &Option<TypedExpr>,
    ) -> Option<(Vec<TypedExpr>, Vec<TypedExpr>)> {
        let condition = condition.as_ref()?;

        let mut left_keys = Vec::new();
        let mut right_keys = Vec::new();

        Self::collect_equality_keys(&condition.expr, &mut left_keys, &mut right_keys)?;

        if left_keys.is_empty() {
            return None;
        }

        Some((left_keys, right_keys))
    }

    /// Recursively collect equality join keys from an expression
    ///
    /// Handles:
    /// - BinaryOp::Eq: Extract left and right columns
    /// - BinaryOp::And: Recursively process both sides
    /// - Other operators: Return None (not supported for HashJoin)
    pub(super) fn collect_equality_keys(
        expr: &Expr,
        left_keys: &mut Vec<TypedExpr>,
        right_keys: &mut Vec<TypedExpr>,
    ) -> Option<()> {
        match expr {
            Expr::BinaryOp { left, op, right } => match op {
                BinaryOperator::Eq => {
                    // Check if this is a column = column comparison
                    if let (
                        Expr::Column {
                            table: left_table,
                            column: left_column,
                        },
                        Expr::Column {
                            table: right_table,
                            column: right_column,
                        },
                    ) = (&left.expr, &right.expr)
                    {
                        // Ensure columns are from different tables
                        if left_table != right_table {
                            left_keys.push(left.as_ref().clone());
                            right_keys.push(right.as_ref().clone());
                            return Some(());
                        }
                    }

                    // Also support right = left order
                    if let (
                        Expr::Column {
                            table: right_table,
                            column: right_column,
                        },
                        Expr::Column {
                            table: left_table,
                            column: left_column,
                        },
                    ) = (&right.expr, &left.expr)
                    {
                        if left_table != right_table {
                            left_keys.push(left.as_ref().clone());
                            right_keys.push(right.as_ref().clone());
                            return Some(());
                        }
                    }

                    // Not a simple column-to-column comparison
                    None
                }
                BinaryOperator::And => {
                    // For AND, we can collect keys from both sides
                    Self::collect_equality_keys(&left.expr, left_keys, right_keys)?;
                    Self::collect_equality_keys(&right.expr, left_keys, right_keys)?;
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
}
