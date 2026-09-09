//! Expression analysis
//!
//! This module handles the analysis and type-checking of SQL expressions including:
//! - Identifiers and column references
//! - Literals and values
//! - CASE expressions
//! - CAST expressions
//! - IS NULL / IS NOT NULL
//! - BETWEEN, IN, LIKE, ILIKE
//! - Nested expressions
//! - Interval expressions
//! - Subquery expressions
//! - $.column.path JSON access syntax
//!
//! # Module Organization
//!
//! The expression analysis is organized into focused submodules:
//!
//! - `identifiers` - Identifier and column reference analysis
//! - `literals` - Literal value analysis
//! - `comparisons` - BETWEEN, IN, LIKE, ILIKE analysis
//! - `case_expr` - CASE expression analysis
//! - `cast` - CAST expression and type conversion
//! - `interval` - INTERVAL expression analysis
//! - `subquery` - IN subquery analysis
//! - `dollar_dot` - $.column.path JSON path syntax

mod case_expr;
mod cast;
mod comparisons;
mod dollar_dot;
mod identifiers;
mod interval;
mod literals;
mod special_forms;
mod subquery;

use super::{AnalyzerContext, Result};
use crate::analyzer::{
    error::AnalysisError,
    typed_expr::{BinaryOperator, Expr, TypedExpr},
    types::DataType,
};
use sqlparser::ast::Expr as SqlExpr;

impl<'a> AnalyzerContext<'a> {
    /// Main entry point for expression analysis
    pub(super) fn analyze_expr(&self, expr: &SqlExpr) -> Result<TypedExpr> {
        match expr {
            SqlExpr::Identifier(ident) => self.analyze_identifier(ident),

            SqlExpr::CompoundIdentifier(idents) => self.analyze_compound_identifier(idents),

            SqlExpr::Value(value_with_span) => self.analyze_value(&value_with_span.value),

            SqlExpr::BinaryOp { left, op, right } => self.analyze_binary_op(left, op, right),

            SqlExpr::UnaryOp { op, expr } => self.analyze_unary_op(op, expr),

            SqlExpr::Function(func) => self.analyze_function(func),

            SqlExpr::Cast {
                expr,
                data_type,
                format: _,
                kind: _,
            } => self.analyze_cast(expr, data_type),

            SqlExpr::IsNull(expr) => {
                let typed_expr = self.analyze_expr(expr)?;
                Ok(TypedExpr::new(
                    Expr::IsNull {
                        expr: Box::new(typed_expr),
                    },
                    DataType::Boolean,
                ))
            }

            SqlExpr::IsNotNull(expr) => {
                let typed_expr = self.analyze_expr(expr)?;
                Ok(TypedExpr::new(
                    Expr::IsNotNull {
                        expr: Box::new(typed_expr),
                    },
                    DataType::Boolean,
                ))
            }

            SqlExpr::Between {
                expr,
                negated,
                low,
                high,
            } => self.analyze_between(expr, *negated, low, high),

            SqlExpr::InList {
                expr,
                list,
                negated,
            } => self.analyze_in_list(expr, list, *negated),

            SqlExpr::Like {
                negated,
                expr,
                pattern,
                escape_char: _,
                ..
            } => self.analyze_like(expr, pattern, *negated),

            SqlExpr::ILike {
                negated,
                expr,
                pattern,
                escape_char: _,
                ..
            } => self.analyze_ilike(expr, pattern, *negated),

            SqlExpr::Nested(expr) => self.analyze_expr(expr),

            // SQL-standard call syntax that the parser gives its own AST node.
            // Each is lowered onto the named function that implements it, so
            // there is one type-check and one kernel per function rather than
            // one per spelling. See `special_forms`.
            SqlExpr::Substring {
                expr,
                substring_from,
                substring_for,
                ..
            } => self.analyze_substring(expr, substring_from.as_deref(), substring_for.as_deref()),

            SqlExpr::Trim {
                expr,
                trim_where,
                trim_what,
                trim_characters,
            } => self.analyze_trim(
                expr,
                trim_where.as_ref(),
                trim_what.as_deref(),
                trim_characters.as_deref(),
            ),

            SqlExpr::Extract { field, expr, .. } => self.analyze_extract(field, expr),

            SqlExpr::Position { expr, r#in } => self.analyze_position(expr, r#in),

            SqlExpr::Ceil { expr, field } => self.analyze_ceil_floor("CEIL", expr, field),

            SqlExpr::Floor { expr, field } => self.analyze_ceil_floor("FLOOR", expr, field),

            SqlExpr::TypedString(typed) => self.analyze_typed_string(typed),

            SqlExpr::Case {
                operand,
                conditions,
                else_result,
                ..
            } => self.analyze_case(operand, conditions, else_result),

            SqlExpr::Interval(interval) => self.analyze_interval(interval),

            SqlExpr::InSubquery {
                expr,
                subquery,
                negated,
            } => self.analyze_in_subquery(expr, subquery, *negated),

            SqlExpr::Exists { subquery, negated } => self.analyze_exists(subquery, *negated),

            SqlExpr::Subquery(subquery) => self.analyze_scalar_subquery(subquery),

            SqlExpr::AnyOp {
                left,
                compare_op,
                right,
                is_some: _,
            } => self.analyze_quantified(left, compare_op, right, false),

            SqlExpr::AllOp {
                left,
                compare_op,
                right,
            } => self.analyze_quantified(left, compare_op, right, true),

            SqlExpr::SimilarTo {
                negated,
                expr,
                pattern,
                escape_char: _,
            } => self.analyze_regex(expr, pattern, false, *negated, true),

            SqlExpr::CompoundFieldAccess { root, access_chain } => {
                self.analyze_compound_field_access(root, access_chain)
            }

            // Standard SQL null-safe comparison (SQL:1999). Desugared into
            // existing IS NULL / equality constructs so the executor needs no
            // dedicated operator.
            SqlExpr::IsDistinctFrom(left, right) => {
                self.analyze_is_distinct_from(left, right, false)
            }
            SqlExpr::IsNotDistinctFrom(left, right) => {
                self.analyze_is_distinct_from(left, right, true)
            }

            // `ARRAY[0.1, 0.2, ...]` — the shape the FUNCTIONS runtime renders a
            // bound JSON array into (`callbacks::sql_params::format_value`).
            // An all-numeric one is a vector literal; anything else is still
            // unsupported, so this widens nothing but the vector case.
            //
            // The SQL/HTTP substituter renders the same bound array as the TEXT
            // `'[0.1,0.2]'` instead, which `vector_literal::parse_vector_text`
            // recognises at the call site. Two renderings, one meaning — which
            // is the point: before this, one surface embedded the string and the
            // other returned `UnsupportedExpression`.
            SqlExpr::Array(array) => self.analyze_array(&array.elem, expr),

            _ => Err(AnalysisError::UnsupportedExpression(
                describe_unsupported_expr(expr),
            )),
        }
    }

    /// An all-numeric `ARRAY[...]` becomes a [`Literal::Vector`].
    ///
    /// Deliberately narrow. A general SQL array type does not exist in this
    /// analyzer, and inventing one here to carry a vector would be a second
    /// notion of "a list of numbers" beside `Literal::Vector`. A non-numeric or
    /// empty array therefore keeps the error it had before.
    fn analyze_array(&self, elements: &[SqlExpr], original: &SqlExpr) -> Result<TypedExpr> {
        use crate::analyzer::typed_expr::Literal;

        let mut values: Vec<f32> = Vec::with_capacity(elements.len());
        for element in elements {
            let typed = self.analyze_expr(element)?;
            let value = match &typed.expr {
                Expr::Literal(Literal::Double(v)) => *v,
                Expr::Literal(Literal::Int(v)) => *v as f64,
                Expr::Literal(Literal::BigInt(v)) => *v as f64,
                _ => {
                    return Err(AnalysisError::UnsupportedExpression(format!(
                        "{:?}",
                        original
                    )))
                }
            };
            if !value.is_finite() {
                return Err(AnalysisError::UnsupportedExpression(format!(
                    "{:?}",
                    original
                )));
            }
            values.push(value as f32);
        }

        if values.is_empty() {
            return Err(AnalysisError::UnsupportedExpression(format!(
                "{:?}",
                original
            )));
        }

        let len = values.len();
        Ok(TypedExpr::new(
            Expr::Literal(Literal::Vector(values)),
            DataType::Vector(len),
        ))
    }

    /// Desugar `a IS [NOT] DISTINCT FROM b` into a NULL-safe boolean expression
    /// built from `IS NULL` / `IS NOT NULL` and `=`/`<>`. RaisinDB evaluates a
    /// comparison with a NULL operand as false, which makes these forms exact:
    ///
    /// - `a IS DISTINCT FROM b`  ⇒ `(a IS NULL AND b IS NOT NULL)
    ///                              OR (a IS NOT NULL AND b IS NULL)
    ///                              OR (a <> b)`
    /// - `a IS NOT DISTINCT FROM b` ⇒ `(a IS NULL AND b IS NULL)
    ///                              OR (a IS NOT NULL AND b IS NOT NULL AND a = b)`
    fn analyze_is_distinct_from(
        &self,
        left: &SqlExpr,
        right: &SqlExpr,
        negated: bool,
    ) -> Result<TypedExpr> {
        let a = self.analyze_expr(left)?;
        let b = self.analyze_expr(right)?;

        let boolean = |e: Expr| TypedExpr::new(e, DataType::Boolean);
        let bin = |l: TypedExpr, op: BinaryOperator, r: TypedExpr| {
            TypedExpr::new(
                Expr::BinaryOp {
                    left: Box::new(l),
                    op,
                    right: Box::new(r),
                },
                DataType::Boolean,
            )
        };
        let is_null = |e: TypedExpr| boolean(Expr::IsNull { expr: Box::new(e) });
        let is_not_null = |e: TypedExpr| boolean(Expr::IsNotNull { expr: Box::new(e) });

        let result = if negated {
            // a IS NOT DISTINCT FROM b
            let both_null = bin(is_null(a.clone()), BinaryOperator::And, is_null(b.clone()));
            let both_present = bin(
                is_not_null(a.clone()),
                BinaryOperator::And,
                is_not_null(b.clone()),
            );
            let both_equal = bin(
                both_present,
                BinaryOperator::And,
                bin(a, BinaryOperator::Eq, b),
            );
            bin(both_null, BinaryOperator::Or, both_equal)
        } else {
            // a IS DISTINCT FROM b
            let a_null_b_present = bin(
                is_null(a.clone()),
                BinaryOperator::And,
                is_not_null(b.clone()),
            );
            let a_present_b_null = bin(
                is_not_null(a.clone()),
                BinaryOperator::And,
                is_null(b.clone()),
            );
            let exactly_one_null = bin(a_null_b_present, BinaryOperator::Or, a_present_b_null);
            let both_present_unequal = bin(a, BinaryOperator::NotEq, b);
            bin(exactly_one_null, BinaryOperator::Or, both_present_unequal)
        };

        Ok(result)
    }
}

/// Name the SQL construct that is not supported, followed by its SQL text
/// (truncated), instead of dumping the parser's AST.
pub(in crate::analyzer) fn describe_unsupported_expr(expr: &SqlExpr) -> String {
    let construct = match expr {
        SqlExpr::Array(_) => "ARRAY literal (only numeric vector literals and ANY/ALL arrays)",
        SqlExpr::Tuple(_) => "row/tuple value",
        SqlExpr::GroupingSets(_) => "GROUPING SETS",
        SqlExpr::Cube(_) => "CUBE",
        SqlExpr::Rollup(_) => "ROLLUP",
        SqlExpr::Collate { .. } => "COLLATE",
        SqlExpr::AtTimeZone { .. } => "AT TIME ZONE",
        SqlExpr::Overlay { .. } => "OVERLAY",
        SqlExpr::Convert { .. } => "CONVERT",
        SqlExpr::TypedString { .. } => "typed string literal",
        SqlExpr::Dictionary(_) | SqlExpr::Map(_) => "map/dictionary literal",
        SqlExpr::Lambda(_) => "lambda expression",
        SqlExpr::MatchAgainst { .. } => "MATCH ... AGAINST",
        SqlExpr::Struct { .. } => "STRUCT literal",
        SqlExpr::Named { .. } => "named argument",
        SqlExpr::IsTrue(_)
        | SqlExpr::IsNotTrue(_)
        | SqlExpr::IsFalse(_)
        | SqlExpr::IsNotFalse(_)
        | SqlExpr::IsUnknown(_)
        | SqlExpr::IsNotUnknown(_) => "IS [NOT] TRUE/FALSE/UNKNOWN",
        SqlExpr::IsNormalized { .. } => "IS NORMALIZED",
        SqlExpr::RLike { .. } => "RLIKE / REGEXP (use ~ or SIMILAR TO)",
        SqlExpr::Position { .. } => "POSITION",
        SqlExpr::Substring { .. } => "SUBSTRING",
        SqlExpr::Trim { .. } => "TRIM",
        SqlExpr::Extract { .. } => "EXTRACT",
        SqlExpr::Ceil { .. } => "CEIL",
        SqlExpr::Floor { .. } => "FLOOR",
        SqlExpr::Prefixed { .. } => "prefixed literal",
        SqlExpr::OuterJoin(_) => "Oracle (+) outer join",
        SqlExpr::Prior(_) => "PRIOR",
        SqlExpr::Wildcard(_) | SqlExpr::QualifiedWildcard(..) => "wildcard in this position",
        _ => "expression",
    };
    let mut text = expr.to_string();
    if text.len() > 120 {
        text.truncate(117);
        text.push_str("...");
    }
    format!("{construct}: `{text}`")
}
