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
    typed_expr::{Expr, TypedExpr},
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

            _ => Err(AnalysisError::UnsupportedExpression(format!("{:?}", expr))),
        }
    }
}
