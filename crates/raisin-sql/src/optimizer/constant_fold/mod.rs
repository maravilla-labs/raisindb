//! Constant Folding Optimization
//!
//! NOTE: mod.rs exceeds 300 lines because fold_constants is a single match
//! expression over all Expr variants (idiomatic Rust AST visitor pattern).
//! The helper functions (binary, unary, cast, functions) are in submodules.
//!
//! Evaluates deterministic functions with constant arguments at compile time.
//!
//! # Examples
//!
//! - `DEPTH('/content/blog')` → `2`
//! - `PARENT('/content/blog/post1')` → `'/content/blog'`
//! - `1 + 2` → `3`
//! - `LOWER('HELLO')` → `'hello'`
//!
//! # Benefits
//!
//! - Reduces runtime computation
//! - Enables further optimizations (e.g., index selection)
//! - Simplifies query plans for debugging

mod binary;
mod cast;
mod functions;
mod unary;

use crate::analyzer::{DataType, Expr, Literal, TypedExpr};

use binary::fold_binary_op;
use cast::fold_cast;
use functions::fold_function;
use unary::fold_unary_op;

/// Apply constant folding to an expression tree
///
/// Recursively traverses the expression and evaluates any deterministic
/// operations with constant operands.
pub fn fold_constants(expr: TypedExpr) -> TypedExpr {
    match expr.expr {
        // Recursively fold binary operations
        Expr::BinaryOp { left, op, right } => {
            let left_folded = fold_constants(*left);
            let right_folded = fold_constants(*right);

            // Try to fold if both sides are literals
            if let (Expr::Literal(left_lit), Expr::Literal(right_lit)) =
                (&left_folded.expr, &right_folded.expr)
            {
                if let Some(result) = fold_binary_op(left_lit, op, right_lit) {
                    return result;
                }
            }

            // Can't fold, reconstruct with folded children
            TypedExpr::new(
                Expr::BinaryOp {
                    left: Box::new(left_folded),
                    op,
                    right: Box::new(right_folded),
                },
                expr.data_type,
            )
        }

        // Recursively fold unary operations
        Expr::UnaryOp { op, expr: inner } => {
            let inner_folded = fold_constants(*inner);

            // Try to fold if operand is literal
            if let Expr::Literal(lit) = &inner_folded.expr {
                if let Some(result) = fold_unary_op(op, lit) {
                    return result;
                }
            }

            // Can't fold, reconstruct with folded child
            TypedExpr::new(
                Expr::UnaryOp {
                    op,
                    expr: Box::new(inner_folded),
                },
                expr.data_type,
            )
        }

        // Fold deterministic functions with literal arguments
        Expr::Function {
            name,
            args,
            signature,
            filter,
        } => {
            // Recursively fold arguments
            let folded_args: Vec<TypedExpr> = args.into_iter().map(fold_constants).collect();

            // Recursively fold filter if present
            let folded_filter = filter.map(|f| Box::new(fold_constants(*f)));

            // Check if function is deterministic and all args are literals
            if signature.is_deterministic
                && folded_args
                    .iter()
                    .all(|a| matches!(a.expr, Expr::Literal(_)))
            {
                if let Some(result) = fold_function(&name, &folded_args) {
                    return result;
                }
            }

            // Can't fold, reconstruct with folded args
            TypedExpr::new(
                Expr::Function {
                    name,
                    args: folded_args,
                    signature,
                    filter: folded_filter,
                },
                expr.data_type,
            )
        }

        // Recursively fold other expression types
        Expr::Cast {
            expr: inner,
            target_type,
        } => {
            let inner_folded = fold_constants(*inner);

            // Try to fold cast of literal
            if let Expr::Literal(lit) = &inner_folded.expr {
                if let Some(result) = fold_cast(lit, &target_type) {
                    return result;
                }
            }

            TypedExpr::new(
                Expr::Cast {
                    expr: Box::new(inner_folded),
                    target_type,
                },
                expr.data_type,
            )
        }

        Expr::IsNull { expr: inner } => {
            let inner_folded = fold_constants(*inner);

            // Fold if operand is literal
            if let Expr::Literal(Literal::Null) = &inner_folded.expr {
                return TypedExpr::literal(Literal::Boolean(true));
            } else if matches!(inner_folded.expr, Expr::Literal(_)) {
                return TypedExpr::literal(Literal::Boolean(false));
            }

            TypedExpr::new(
                Expr::IsNull {
                    expr: Box::new(inner_folded),
                },
                expr.data_type,
            )
        }

        Expr::IsNotNull { expr: inner } => {
            let inner_folded = fold_constants(*inner);

            // Fold if operand is literal
            if let Expr::Literal(Literal::Null) = &inner_folded.expr {
                return TypedExpr::literal(Literal::Boolean(false));
            } else if matches!(inner_folded.expr, Expr::Literal(_)) {
                return TypedExpr::literal(Literal::Boolean(true));
            }

            TypedExpr::new(
                Expr::IsNotNull {
                    expr: Box::new(inner_folded),
                },
                expr.data_type,
            )
        }

        Expr::Between {
            expr: inner,
            low,
            high,
        } => {
            let inner_folded = fold_constants(*inner);
            let low_folded = fold_constants(*low);
            let high_folded = fold_constants(*high);

            TypedExpr::new(
                Expr::Between {
                    expr: Box::new(inner_folded),
                    low: Box::new(low_folded),
                    high: Box::new(high_folded),
                },
                expr.data_type,
            )
        }

        Expr::InList {
            expr: inner,
            list,
            negated,
        } => {
            let inner_folded = fold_constants(*inner);
            let list_folded: Vec<TypedExpr> = list.into_iter().map(fold_constants).collect();

            TypedExpr::new(
                Expr::InList {
                    expr: Box::new(inner_folded),
                    list: list_folded,
                    negated,
                },
                expr.data_type,
            )
        }

        Expr::InSubquery {
            expr: inner,
            subquery,
            subquery_type,
            negated,
        } => {
            // Fold only the left expression, subquery is already analyzed
            let inner_folded = fold_constants(*inner);

            TypedExpr::new(
                Expr::InSubquery {
                    expr: Box::new(inner_folded),
                    subquery,
                    subquery_type,
                    negated,
                },
                expr.data_type,
            )
        }

        Expr::Like {
            expr: inner,
            pattern,
            negated,
        } => {
            let inner_folded = fold_constants(*inner);
            let pattern_folded = fold_constants(*pattern);

            TypedExpr::new(
                Expr::Like {
                    expr: Box::new(inner_folded),
                    pattern: Box::new(pattern_folded),
                    negated,
                },
                expr.data_type,
            )
        }

        Expr::ILike {
            expr: inner,
            pattern,
            negated,
        } => {
            let inner_folded = fold_constants(*inner);
            let pattern_folded = fold_constants(*pattern);

            TypedExpr::new(
                Expr::ILike {
                    expr: Box::new(inner_folded),
                    pattern: Box::new(pattern_folded),
                    negated,
                },
                expr.data_type,
            )
        }

        // JSON operations - fold child expressions using shared helper
        Expr::JsonExtract { object, key } => {
            fold_json_pair(*object, *key, expr.data_type, |o, k| Expr::JsonExtract {
                object: Box::new(o),
                key: Box::new(k),
            })
        }
        Expr::JsonExtractText { object, key } => {
            fold_json_pair(*object, *key, expr.data_type, |o, k| {
                Expr::JsonExtractText {
                    object: Box::new(o),
                    key: Box::new(k),
                }
            })
        }
        Expr::JsonContains { object, pattern } => {
            fold_json_pair(*object, *pattern, expr.data_type, |o, p| {
                Expr::JsonContains {
                    object: Box::new(o),
                    pattern: Box::new(p),
                }
            })
        }
        Expr::JsonKeyExists { object, key } => {
            fold_json_pair(*object, *key, expr.data_type, |o, k| Expr::JsonKeyExists {
                object: Box::new(o),
                key: Box::new(k),
            })
        }
        Expr::JsonAnyKeyExists { object, keys } => {
            fold_json_pair(*object, *keys, expr.data_type, |o, k| {
                Expr::JsonAnyKeyExists {
                    object: Box::new(o),
                    keys: Box::new(k),
                }
            })
        }
        Expr::JsonAllKeyExists { object, keys } => {
            fold_json_pair(*object, *keys, expr.data_type, |o, k| {
                Expr::JsonAllKeyExists {
                    object: Box::new(o),
                    keys: Box::new(k),
                }
            })
        }
        Expr::JsonExtractPath { object, path } => {
            fold_json_pair(*object, *path, expr.data_type, |o, p| {
                Expr::JsonExtractPath {
                    object: Box::new(o),
                    path: Box::new(p),
                }
            })
        }
        Expr::JsonExtractPathText { object, path } => {
            fold_json_pair(*object, *path, expr.data_type, |o, p| {
                Expr::JsonExtractPathText {
                    object: Box::new(o),
                    path: Box::new(p),
                }
            })
        }
        Expr::JsonRemove { object, key } => {
            fold_json_pair(*object, *key, expr.data_type, |o, k| Expr::JsonRemove {
                object: Box::new(o),
                key: Box::new(k),
            })
        }
        Expr::JsonRemoveAtPath { object, path } => {
            fold_json_pair(*object, *path, expr.data_type, |o, p| {
                Expr::JsonRemoveAtPath {
                    object: Box::new(o),
                    path: Box::new(p),
                }
            })
        }
        Expr::JsonPathMatch { object, path } => {
            fold_json_pair(*object, *path, expr.data_type, |o, p| Expr::JsonPathMatch {
                object: Box::new(o),
                path: Box::new(p),
            })
        }
        Expr::JsonPathExists { object, path } => {
            fold_json_pair(*object, *path, expr.data_type, |o, p| {
                Expr::JsonPathExists {
                    object: Box::new(o),
                    path: Box::new(p),
                }
            })
        }

        // Window functions cannot be constant-folded (they depend on row context)
        Expr::Window {
            function,
            partition_by,
            order_by,
            frame,
        } => {
            // Recursively fold partition_by and order_by expressions
            let partition_by_folded: Vec<TypedExpr> =
                partition_by.into_iter().map(fold_constants).collect();
            let order_by_folded: Vec<(TypedExpr, bool)> = order_by
                .into_iter()
                .map(|(expr, desc)| (fold_constants(expr), desc))
                .collect();

            TypedExpr::new(
                Expr::Window {
                    function,
                    partition_by: partition_by_folded,
                    order_by: order_by_folded,
                    frame,
                },
                expr.data_type,
            )
        }

        // CASE expressions - recursively fold conditions and results
        Expr::Case {
            conditions,
            else_expr,
        } => {
            // Fold all conditions and results
            let conditions_folded: Vec<(TypedExpr, TypedExpr)> = conditions
                .into_iter()
                .map(|(cond, result)| (fold_constants(cond), fold_constants(result)))
                .collect();

            let else_folded = else_expr.map(|e| Box::new(fold_constants(*e)));

            // Try to evaluate if condition is a literal
            for (cond, result) in &conditions_folded {
                if let Expr::Literal(Literal::Boolean(true)) = &cond.expr {
                    // Condition is always true - return this result
                    return result.clone();
                }
            }

            // If ELSE is present and all conditions are false literals, return ELSE
            if let Some(else_result) = &else_folded {
                let all_false = conditions_folded
                    .iter()
                    .all(|(cond, _)| matches!(&cond.expr, Expr::Literal(Literal::Boolean(false))));
                if all_false {
                    return *else_result.clone();
                }
            }

            // Can't fully fold, reconstruct with folded children
            TypedExpr::new(
                Expr::Case {
                    conditions: conditions_folded,
                    else_expr: else_folded,
                },
                expr.data_type,
            )
        }

        // Literals and columns can't be folded further
        Expr::Literal(_) | Expr::Column { .. } => expr,
    }
}

/// Fold a pair of child expressions and reconstruct the parent.
///
/// All JSON expression variants share the same fold pattern: recursively
/// fold both children, then reconstruct with the provided constructor.
fn fold_json_pair(
    first: TypedExpr,
    second: TypedExpr,
    data_type: DataType,
    constructor: impl FnOnce(TypedExpr, TypedExpr) -> Expr,
) -> TypedExpr {
    let first_folded = fold_constants(first);
    let second_folded = fold_constants(second);
    TypedExpr::new(constructor(first_folded, second_folded), data_type)
}

#[cfg(test)]
mod tests;
