//! Expression helpers for the plan builder.

use super::PlanBuilder;
use crate::logical_plan::operators::WindowExpr;

impl<'a> PlanBuilder<'a> {
    /// Check if an expression contains a window function
    pub(crate) fn contains_window_function(expr: &crate::analyzer::TypedExpr) -> bool {
        use crate::analyzer::Expr;

        match &expr.expr {
            Expr::Window { .. } => true,
            Expr::BinaryOp { left, right, .. } => {
                Self::contains_window_function(left) || Self::contains_window_function(right)
            }
            Expr::UnaryOp { expr, .. } => Self::contains_window_function(expr),
            Expr::Cast { expr, .. } => Self::contains_window_function(expr),
            Expr::IsNull { expr } | Expr::IsNotNull { expr } => {
                Self::contains_window_function(expr)
            }
            Expr::Between { expr, low, high } => {
                Self::contains_window_function(expr)
                    || Self::contains_window_function(low)
                    || Self::contains_window_function(high)
            }
            Expr::InList { expr, list, .. } => {
                Self::contains_window_function(expr)
                    || list.iter().any(Self::contains_window_function)
            }
            Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
                Self::contains_window_function(expr) || Self::contains_window_function(pattern)
            }
            Expr::JsonExtract { object, key }
            | Expr::JsonExtractText { object, key }
            | Expr::JsonContains {
                object,
                pattern: key,
            } => Self::contains_window_function(object) || Self::contains_window_function(key),
            Expr::Function { args, .. } => args.iter().any(Self::contains_window_function),
            _ => false,
        }
    }

    /// Extract all window functions from an expression, replacing them with column references
    ///
    /// Returns: (rewritten_expression, extracted_window_functions)
    pub(crate) fn extract_window_functions(
        expr: &crate::analyzer::TypedExpr,
    ) -> (crate::analyzer::TypedExpr, Vec<WindowExpr>) {
        use crate::analyzer::Expr;

        let mut extracted = Vec::new();

        let new_expr = match &expr.expr {
            // Found a window function - extract it
            Expr::Window {
                function,
                partition_by,
                order_by,
                frame,
            } => {
                // Generate auto-alias for this window function
                let window_alias = Self::derive_column_name(expr);

                // Add to extracted list
                extracted.push(WindowExpr {
                    function: function.clone(),
                    partition_by: partition_by.clone(),
                    order_by: order_by.clone(),
                    frame: frame.clone(),
                    alias: window_alias.clone(),
                    return_type: expr.data_type.clone(),
                });

                // Replace with column reference
                crate::analyzer::TypedExpr {
                    expr: Expr::Column {
                        table: "".to_string(), // Unqualified column reference
                        column: window_alias,
                    },
                    data_type: expr.data_type.clone(),
                }
            }

            // Recursively process binary operations
            Expr::BinaryOp { left, op, right } => {
                let (new_left, mut left_windows) = Self::extract_window_functions(left);
                let (new_right, mut right_windows) = Self::extract_window_functions(right);
                extracted.append(&mut left_windows);
                extracted.append(&mut right_windows);

                crate::analyzer::TypedExpr {
                    expr: Expr::BinaryOp {
                        left: Box::new(new_left),
                        op: *op,
                        right: Box::new(new_right),
                    },
                    data_type: expr.data_type.clone(),
                }
            }

            // Recursively process unary operations
            Expr::UnaryOp { op, expr: inner } => {
                let (new_inner, mut inner_windows) = Self::extract_window_functions(inner);
                extracted.append(&mut inner_windows);

                crate::analyzer::TypedExpr {
                    expr: Expr::UnaryOp {
                        op: *op,
                        expr: Box::new(new_inner),
                    },
                    data_type: expr.data_type.clone(),
                }
            }

            // Recursively process function arguments
            Expr::Function {
                name,
                args,
                signature,
                filter,
            } => {
                let mut new_args = Vec::new();
                for arg in args {
                    let (new_arg, mut arg_windows) = Self::extract_window_functions(arg);
                    extracted.append(&mut arg_windows);
                    new_args.push(new_arg);
                }

                // Also process filter if present
                let new_filter = if let Some(f) = filter {
                    let (new_f, mut f_windows) = Self::extract_window_functions(f);
                    extracted.append(&mut f_windows);
                    Some(Box::new(new_f))
                } else {
                    None
                };

                crate::analyzer::TypedExpr {
                    expr: Expr::Function {
                        name: name.clone(),
                        args: new_args,
                        signature: signature.clone(),
                        filter: new_filter,
                    },
                    data_type: expr.data_type.clone(),
                }
            }

            // Recursively process cast
            Expr::Cast {
                expr: inner,
                target_type,
            } => {
                let (new_inner, mut inner_windows) = Self::extract_window_functions(inner);
                extracted.append(&mut inner_windows);

                crate::analyzer::TypedExpr {
                    expr: Expr::Cast {
                        expr: Box::new(new_inner),
                        target_type: target_type.clone(),
                    },
                    data_type: expr.data_type.clone(),
                }
            }

            // No window functions in this branch - return as is
            _ => expr.clone(),
        };

        (new_expr, extracted)
    }

    /// Derive a column name from an expression
    pub(crate) fn derive_column_name(expr: &crate::analyzer::TypedExpr) -> String {
        use crate::analyzer::Expr;

        match &expr.expr {
            Expr::Column { column, .. } => column.clone(),
            Expr::Function { name, .. } => name.to_lowercase(),
            Expr::Literal(_) => "?column?".to_string(),
            Expr::BinaryOp { .. } => "?column?".to_string(),
            Expr::UnaryOp { .. } => "?column?".to_string(),
            Expr::Cast { expr, .. } => Self::derive_column_name(expr),
            Expr::IsNull { .. } => "?column?".to_string(),
            Expr::IsNotNull { .. } => "?column?".to_string(),
            Expr::Between { .. } => "?column?".to_string(),
            Expr::InList { .. } => "?column?".to_string(),
            Expr::InSubquery { .. } => "?column?".to_string(),
            Expr::Like { .. } | Expr::ILike { .. } => "?column?".to_string(),
            Expr::JsonExtract { .. } => "?column?".to_string(),
            Expr::JsonExtractText { .. } => "?column?".to_string(),
            Expr::JsonContains { .. } => "?column?".to_string(),
            Expr::JsonKeyExists { .. } => "?column?".to_string(),
            Expr::JsonAnyKeyExists { .. } => "?column?".to_string(),
            Expr::JsonAllKeyExists { .. } => "?column?".to_string(),
            Expr::JsonExtractPath { .. } => "?column?".to_string(),
            Expr::JsonExtractPathText { .. } => "?column?".to_string(),
            Expr::JsonRemove { .. } => "?column?".to_string(),
            Expr::JsonRemoveAtPath { .. } => "?column?".to_string(),
            Expr::JsonPathMatch { .. } => "?column?".to_string(),
            Expr::JsonPathExists { .. } => "?column?".to_string(),
            Expr::Window { function, .. } => {
                // Derive name from window function type
                match function {
                    crate::analyzer::WindowFunction::RowNumber => "row_number".to_string(),
                    crate::analyzer::WindowFunction::Rank => "rank".to_string(),
                    crate::analyzer::WindowFunction::DenseRank => "dense_rank".to_string(),
                    crate::analyzer::WindowFunction::Count => "count".to_string(),
                    crate::analyzer::WindowFunction::Sum(_) => "sum".to_string(),
                    crate::analyzer::WindowFunction::Avg(_) => "avg".to_string(),
                    crate::analyzer::WindowFunction::Min(_) => "min".to_string(),
                    crate::analyzer::WindowFunction::Max(_) => "max".to_string(),
                }
            }
            Expr::Case { .. } => "case".to_string(),
        }
    }

    /// Check if two expressions are structurally equal
    /// Used to match SELECT expressions with GROUP BY expressions
    pub(crate) fn exprs_match(
        left: &crate::analyzer::TypedExpr,
        right: &crate::analyzer::TypedExpr,
    ) -> bool {
        use crate::analyzer::Expr;

        match (&left.expr, &right.expr) {
            // Columns match if table and column name are the same
            (
                Expr::Column {
                    table: t1,
                    column: c1,
                },
                Expr::Column {
                    table: t2,
                    column: c2,
                },
            ) => t1 == t2 && c1 == c2,

            // Functions match if name and args match
            (
                Expr::Function {
                    name: n1, args: a1, ..
                },
                Expr::Function {
                    name: n2, args: a2, ..
                },
            ) => {
                n1.eq_ignore_ascii_case(n2)
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(l, r)| Self::exprs_match(l, r))
            }

            // Literals match if equal
            (Expr::Literal(l1), Expr::Literal(l2)) => l1 == l2,

            // Binary ops match if op and operands match
            (
                Expr::BinaryOp {
                    left: l1,
                    op: op1,
                    right: r1,
                },
                Expr::BinaryOp {
                    left: l2,
                    op: op2,
                    right: r2,
                },
            ) => op1 == op2 && Self::exprs_match(l1, l2) && Self::exprs_match(r1, r2),

            // JSON extract text matches
            (
                Expr::JsonExtractText {
                    object: o1,
                    key: k1,
                },
                Expr::JsonExtractText {
                    object: o2,
                    key: k2,
                },
            ) => Self::exprs_match(o1, o2) && Self::exprs_match(k1, k2),

            // Add other expression types as needed
            _ => false,
        }
    }

    /// Generate canonical column name for GROUP BY expression
    /// Must match logic in hash_aggregate.rs extract_column_name()
    pub(crate) fn generate_groupby_column_name(expr: &crate::analyzer::TypedExpr) -> String {
        use crate::analyzer::Expr;

        match &expr.expr {
            Expr::Column { table, column } => {
                format!("{}.{}", table, column)
            }
            Expr::Function { name, args, .. } => {
                let func_name_upper = name.to_uppercase();
                if args.is_empty() {
                    format!("{}()", func_name_upper)
                } else if args.len() == 1 {
                    let arg_name = Self::generate_groupby_column_name(&args[0]);
                    format!("{}({})", func_name_upper, arg_name)
                } else {
                    format!("{}(...)", func_name_upper)
                }
            }
            Expr::JsonExtractText { object, key } => {
                if let Expr::Column { table, column } = &object.expr {
                    if let Expr::Literal(crate::analyzer::Literal::Text(key_str)) = &key.expr {
                        return format!("{}.{}_{}", table, column, key_str);
                    }
                }
                "?column?".to_string()
            }
            _ => "?column?".to_string(),
        }
    }

    /// Rewrite expressions that match GROUP BY expressions to column references
    /// Returns a new expression list with matched expressions replaced
    pub(crate) fn rewrite_groupby_refs(
        exprs: &[crate::analyzer::TypedExpr],
        group_by: &[crate::analyzer::TypedExpr],
    ) -> Vec<crate::analyzer::TypedExpr> {
        use crate::analyzer::Expr;

        exprs
            .iter()
            .map(|expr| {
                // Check if this expression matches any GROUP BY expression
                for group_expr in group_by {
                    if Self::exprs_match(expr, group_expr) {
                        // Match found! Replace with column reference
                        let canonical_name = Self::generate_groupby_column_name(group_expr);
                        return crate::analyzer::TypedExpr::new(
                            Expr::Column {
                                table: "".to_string(),
                                column: canonical_name,
                            },
                            expr.data_type.clone(),
                        );
                    }
                }
                // No match - keep as-is
                expr.clone()
            })
            .collect()
    }

    /// Rewrite ORDER BY expressions to reference projection aliases or GROUP BY columns
    pub(crate) fn rewrite_order_by_expr(
        expr: &crate::analyzer::TypedExpr,
        query: &crate::analyzer::AnalyzedQuery,
        projection_aliases: &[String],
    ) -> crate::analyzer::TypedExpr {
        use crate::analyzer::Expr;

        // First, see if this matches a SELECT projection (to use its alias)
        for (idx, (proj_expr, _)) in query.projection.iter().enumerate() {
            if Self::exprs_match(expr, proj_expr) {
                let alias = projection_aliases[idx].clone();
                return crate::analyzer::TypedExpr::new(
                    Expr::Column {
                        table: "".to_string(),
                        column: alias,
                    },
                    expr.data_type.clone(),
                );
            }
        }

        // Next, check if it matches a GROUP BY expression
        if !query.group_by.is_empty() {
            let rewritten = Self::rewrite_groupby_refs(std::slice::from_ref(expr), &query.group_by);
            if let Some(first) = rewritten.into_iter().next() {
                return first;
            }
        }

        expr.clone()
    }
}
