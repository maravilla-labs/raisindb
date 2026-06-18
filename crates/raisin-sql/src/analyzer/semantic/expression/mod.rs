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

            _ => Err(AnalysisError::UnsupportedExpression(format!("{:?}", expr))),
        }
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
