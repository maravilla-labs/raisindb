//! Vector k-NN search pattern detection
//!
//! Detects ORDER BY vector_distance(col, query) LIMIT k patterns
//! and extracts vector search parameters.

use super::{Expr, LogicalPlan, PhysicalPlanner, ProjectionExpr, TypedExpr, VectorDistanceMetric};
use raisin_sql::analyzer::BinaryOperator;

impl PhysicalPlanner {
    /// Detect vector k-NN search pattern and extract components
    ///
    /// Looks for pattern: ORDER BY (vector_col <op> query_vec) LIMIT k
    /// Returns: (vector_column, query_vector, distance_metric, is_ascending)
    pub(super) fn detect_vector_knn_pattern(
        &self,
        sort_expr: &TypedExpr,
    ) -> Option<(String, TypedExpr, VectorDistanceMetric, bool)> {
        match &sort_expr.expr {
            Expr::BinaryOp { left, op, right } => {
                // Check if this is a vector distance operator
                let metric = match op {
                    BinaryOperator::VectorL2Distance => VectorDistanceMetric::L2,
                    BinaryOperator::VectorCosineDistance => VectorDistanceMetric::Cosine,
                    BinaryOperator::VectorInnerProduct => VectorDistanceMetric::InnerProduct,
                    _ => return None,
                };

                // Extract vector column (left side should be a column reference)
                let vector_column = match &left.expr {
                    Expr::Column { column, .. } => column.clone(),
                    _ => return None,
                };

                // Right side is the query vector expression
                let query_vector = (**right).clone();

                Some((vector_column, query_vector, metric, true)) // ascending = true for k-NN
            }
            // Also handle function-based syntax: VECTOR_L2_DISTANCE(col, query)
            Expr::Function { name, args, .. } => {
                let metric = match name.to_uppercase().as_str() {
                    "VECTOR_L2_DISTANCE" => VectorDistanceMetric::L2,
                    "VECTOR_COSINE_DISTANCE" => VectorDistanceMetric::Cosine,
                    "VECTOR_INNER_PRODUCT" => VectorDistanceMetric::InnerProduct,
                    _ => return None,
                };

                if args.len() != 2 {
                    return None;
                }

                // First arg should be column
                let vector_column = match &args[0].expr {
                    Expr::Column { column, .. } => column.clone(),
                    _ => return None,
                };

                // Second arg is query vector
                let query_vector = args[1].clone();

                Some((vector_column, query_vector, metric, true))
            }
            _ => None,
        }
    }

    /// Extract vector sort expression from a Project node
    ///
    /// Handles pattern: SELECT *, embedding <=> EMBEDDING(...) AS sim FROM t ORDER BY sim
    /// This looks through the Project's computed columns to find one that:
    /// 1. Matches the sort expression (by alias)
    /// 2. Is a vector distance operation
    ///
    /// Returns: (vector_column, query_vector, distance_metric)
    pub(super) fn extract_vector_sort_from_project(
        &self,
        project_exprs: &[ProjectionExpr],
        sort_expr: &TypedExpr,
    ) -> Option<(String, TypedExpr, VectorDistanceMetric, bool)> {
        // Sort expression should be a column reference (the alias from the projection)
        let sort_column = match &sort_expr.expr {
            Expr::Column { column, .. } => column,
            _ => return None,
        };

        // Find the projection expression with matching alias
        for proj in project_exprs {
            if proj.alias == *sort_column {
                // Found the matching projection - check if it's a vector distance expression
                return self.detect_vector_knn_pattern(&proj.expr);
            }
        }

        None
    }

    /// Attempt to recover the alias used for a vector distance expression within upstream plan nodes
    pub(super) fn find_vector_distance_alias(
        &self,
        plan: &LogicalPlan,
        target_column: &str,
        target_query: &TypedExpr,
        target_metric: VectorDistanceMetric,
    ) -> Option<String> {
        match plan {
            LogicalPlan::Project { input, exprs } => {
                for proj in exprs {
                    if let Some((proj_column, proj_query, proj_metric, _)) =
                        self.detect_vector_knn_pattern(&proj.expr)
                    {
                        if proj_column == target_column
                            && proj_metric == target_metric
                            && Self::vector_query_exprs_match(&proj_query, target_query)
                        {
                            return Some(proj.alias.clone());
                        }
                    }
                }

                self.find_vector_distance_alias(
                    input.as_ref(),
                    target_column,
                    target_query,
                    target_metric,
                )
            }
            LogicalPlan::Filter { input, .. } => self.find_vector_distance_alias(
                input.as_ref(),
                target_column,
                target_query,
                target_metric,
            ),
            _ => None,
        }
    }

    /// Check whether two typed expressions represent the same vector query
    pub(super) fn vector_query_exprs_match(left: &TypedExpr, right: &TypedExpr) -> bool {
        Self::exprs_structural_eq(&left.expr, &right.expr)
    }

    /// Structural equality comparison for analyzer expressions focusing on supported vector cases
    pub(super) fn exprs_structural_eq(left: &Expr, right: &Expr) -> bool {
        use Expr::*;

        match (left, right) {
            (Literal(l), Literal(r)) => l == r,
            (
                Function {
                    name: name_l,
                    args: args_l,
                    ..
                },
                Function {
                    name: name_r,
                    args: args_r,
                    ..
                },
            ) => {
                name_l.eq_ignore_ascii_case(name_r)
                    && args_l.len() == args_r.len()
                    && args_l
                        .iter()
                        .zip(args_r.iter())
                        .all(|(l_arg, r_arg)| Self::exprs_structural_eq(&l_arg.expr, &r_arg.expr))
            }
            (
                BinaryOp {
                    left: left_l,
                    op: op_l,
                    right: right_l,
                },
                BinaryOp {
                    left: left_r,
                    op: op_r,
                    right: right_r,
                },
            ) => {
                op_l == op_r
                    && Self::exprs_structural_eq(&left_l.expr, &left_r.expr)
                    && Self::exprs_structural_eq(&right_l.expr, &right_r.expr)
            }
            (
                Column {
                    table: table_l,
                    column: column_l,
                },
                Column {
                    table: table_r,
                    column: column_r,
                },
            ) => table_l == table_r && column_l == column_r,
            (
                Cast {
                    expr: expr_l,
                    target_type: target_l,
                },
                Cast {
                    expr: expr_r,
                    target_type: target_r,
                },
            ) => target_l == target_r && Self::exprs_structural_eq(&expr_l.expr, &expr_r.expr),
            _ => false,
        }
    }
}
