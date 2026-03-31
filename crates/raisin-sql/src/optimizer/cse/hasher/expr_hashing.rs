//! Core expression hashing logic
//!
//! Hashes expression variants (the top-level Expr enum dispatch) and
//! the primary entry point for computing expression hashes.

use crate::analyzer::{typed_expr::*, TypedExpr};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::type_hashing;

/// Computes stable structural hashes for expressions
///
/// The hasher treats expressions with identical structure and values as equal,
/// enabling detection of common subexpressions across a query plan.
pub struct ExprHasher;

impl ExprHasher {
    /// Compute a stable hash for an expression based on its structure
    ///
    /// Two expressions with identical structure and values will hash to the same value.
    /// This enables CSE to identify repeated expressions.
    pub fn hash_expr(expr: &TypedExpr) -> u64 {
        let mut hasher = DefaultHasher::new();
        Self::hash_typed_expr(expr, &mut hasher);
        hasher.finish()
    }

    /// Recursively hash a typed expression
    pub(super) fn hash_typed_expr(expr: &TypedExpr, hasher: &mut DefaultHasher) {
        Self::hash_expr_variant(&expr.expr, hasher);
        type_hashing::hash_data_type(&expr.data_type, hasher);
    }

    /// Hash an expression variant (the core expression structure)
    fn hash_expr_variant(expr: &Expr, hasher: &mut DefaultHasher) {
        match expr {
            Expr::Literal(lit) => {
                0u8.hash(hasher);
                type_hashing::hash_literal(lit, hasher);
            }

            Expr::Column { table, column } => {
                1u8.hash(hasher);
                table.hash(hasher);
                column.hash(hasher);
            }

            Expr::Function {
                name,
                args,
                signature,
                filter,
            } => {
                2u8.hash(hasher);
                name.hash(hasher);
                for arg in args {
                    Self::hash_typed_expr(arg, hasher);
                }
                signature.name.hash(hasher);
                signature.is_deterministic.hash(hasher);
                if let Some(filter_expr) = filter {
                    true.hash(hasher);
                    Self::hash_typed_expr(filter_expr, hasher);
                } else {
                    false.hash(hasher);
                }
            }

            Expr::BinaryOp { left, op, right } => {
                3u8.hash(hasher);

                // For commutative operators, canonicalize by sorting operands
                if Self::is_commutative(op) {
                    let left_hash = Self::hash_expr(left);
                    let right_hash = Self::hash_expr(right);

                    if left_hash <= right_hash {
                        Self::hash_typed_expr(left, hasher);
                        type_hashing::hash_binary_op(op, hasher);
                        Self::hash_typed_expr(right, hasher);
                    } else {
                        Self::hash_typed_expr(right, hasher);
                        type_hashing::hash_binary_op(op, hasher);
                        Self::hash_typed_expr(left, hasher);
                    }
                } else {
                    Self::hash_typed_expr(left, hasher);
                    type_hashing::hash_binary_op(op, hasher);
                    Self::hash_typed_expr(right, hasher);
                }
            }

            Expr::UnaryOp { op, expr } => {
                4u8.hash(hasher);
                type_hashing::hash_unary_op(op, hasher);
                Self::hash_typed_expr(expr, hasher);
            }

            Expr::Cast { expr, target_type } => {
                5u8.hash(hasher);
                Self::hash_typed_expr(expr, hasher);
                type_hashing::hash_data_type(target_type, hasher);
            }

            Expr::IsNull { expr } => {
                6u8.hash(hasher);
                Self::hash_typed_expr(expr, hasher);
            }

            Expr::IsNotNull { expr } => {
                7u8.hash(hasher);
                Self::hash_typed_expr(expr, hasher);
            }

            Expr::Between { expr, low, high } => {
                8u8.hash(hasher);
                Self::hash_typed_expr(expr, hasher);
                Self::hash_typed_expr(low, hasher);
                Self::hash_typed_expr(high, hasher);
            }

            Expr::InList {
                expr,
                list,
                negated,
            } => {
                9u8.hash(hasher);
                Self::hash_typed_expr(expr, hasher);
                negated.hash(hasher);
                for item in list {
                    Self::hash_typed_expr(item, hasher);
                }
            }

            Expr::InSubquery { expr, negated, .. } => {
                100u8.hash(hasher);
                Self::hash_typed_expr(expr, hasher);
                negated.hash(hasher);
            }

            Expr::Like {
                expr,
                pattern,
                negated,
            } => {
                10u8.hash(hasher);
                Self::hash_typed_expr(expr, hasher);
                Self::hash_typed_expr(pattern, hasher);
                negated.hash(hasher);
            }

            Expr::ILike {
                expr,
                pattern,
                negated,
            } => {
                30u8.hash(hasher);
                Self::hash_typed_expr(expr, hasher);
                Self::hash_typed_expr(pattern, hasher);
                negated.hash(hasher);
            }

            Expr::JsonExtract { object, key } => {
                11u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(key, hasher);
            }

            Expr::JsonExtractText { object, key } => {
                12u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(key, hasher);
            }

            Expr::JsonContains { object, pattern } => {
                13u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(pattern, hasher);
            }

            Expr::JsonKeyExists { object, key } => {
                14u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(key, hasher);
            }

            Expr::JsonAnyKeyExists { object, keys } => {
                15u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(keys, hasher);
            }

            Expr::JsonAllKeyExists { object, keys } => {
                16u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(keys, hasher);
            }

            Expr::JsonExtractPath { object, path } => {
                17u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(path, hasher);
            }

            Expr::JsonExtractPathText { object, path } => {
                18u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(path, hasher);
            }

            Expr::JsonRemove { object, key } => {
                19u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(key, hasher);
            }

            Expr::JsonRemoveAtPath { object, path } => {
                20u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(path, hasher);
            }

            Expr::JsonPathMatch { object, path } => {
                21u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(path, hasher);
            }

            Expr::JsonPathExists { object, path } => {
                22u8.hash(hasher);
                Self::hash_typed_expr(object, hasher);
                Self::hash_typed_expr(path, hasher);
            }

            Expr::Window {
                function,
                partition_by,
                order_by,
                frame,
            } => {
                15u8.hash(hasher);
                type_hashing::hash_window_function(function, hasher);
                for expr in partition_by {
                    Self::hash_typed_expr(expr, hasher);
                }
                for (expr, desc) in order_by {
                    Self::hash_typed_expr(expr, hasher);
                    desc.hash(hasher);
                }
                if let Some(f) = frame {
                    true.hash(hasher);
                    type_hashing::hash_window_frame(f, hasher);
                } else {
                    false.hash(hasher);
                }
            }

            Expr::Case {
                conditions,
                else_expr,
            } => {
                16u8.hash(hasher);
                for (cond, result) in conditions {
                    Self::hash_typed_expr(cond, hasher);
                    Self::hash_typed_expr(result, hasher);
                }
                if let Some(else_e) = else_expr {
                    true.hash(hasher);
                    Self::hash_typed_expr(else_e, hasher);
                } else {
                    false.hash(hasher);
                }
            }
        }
    }

    /// Check if a binary operator is commutative
    ///
    /// Commutative operators produce the same result regardless of operand order:
    /// - Arithmetic: +, *
    /// - Comparison: =, !=
    /// - Logical: AND, OR
    fn is_commutative(op: &BinaryOperator) -> bool {
        matches!(
            op,
            BinaryOperator::Add
                | BinaryOperator::Multiply
                | BinaryOperator::Eq
                | BinaryOperator::NotEq
                | BinaryOperator::And
                | BinaryOperator::Or
        )
    }
}
