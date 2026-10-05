//! Literal value analysis
//!
//! This module handles the analysis and type-checking of SQL literal values including:
//! - Numbers (integers, floats)
//! - Strings (single-quoted, double-quoted, dollar-quoted)
//! - Booleans
//! - NULL
//! - Parameters/placeholders

use crate::analyzer::{
    error::AnalysisError,
    semantic::{AnalyzerContext, Result},
    typed_expr::{Literal, TypedExpr},
};
use sqlparser::ast::Value;

impl<'a> AnalyzerContext<'a> {
    /// Analyze a literal value
    pub(in crate::analyzer::semantic) fn analyze_value(&self, value: &Value) -> Result<TypedExpr> {
        if let Value::Placeholder(p) = value {
            // Typed like its bound value's literal in a template; `Unknown`
            // in ordinary SQL (plan Phase 13d).
            return Ok(TypedExpr::new(
                crate::analyzer::Expr::Literal(Literal::Parameter(p.clone())),
                self.placeholder_type(p)?,
            ));
        }
        Ok(TypedExpr::literal(literal_from_sql_value(value)?))
    }
}

/// The literal a SQL value token analyzes to — THE one rule, shared by the
/// analyzer and by parameter binding (`raisin_sql::template`), so a bound
/// parameter is the literal its substituted text would have been.
pub(crate) fn literal_from_sql_value(value: &Value) -> Result<Literal> {
    let literal = match value {
        Value::Number(n, _) => {
            if let Ok(i) = n.parse::<i32>() {
                Literal::Int(i)
            } else if let Ok(i) = n.parse::<i64>() {
                Literal::BigInt(i)
            } else if let Ok(f) = n.parse::<f64>() {
                Literal::Double(f)
            } else {
                return Err(AnalysisError::UnsupportedExpression(format!(
                    "Invalid number: {}",
                    n
                )));
            }
        }
        Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => Literal::Text(s.clone()),
        Value::DollarQuotedString(dqs) => Literal::Text(dqs.value.clone()),
        Value::Placeholder(p) => Literal::Parameter(p.clone()),
        Value::Boolean(b) => Literal::Boolean(*b),
        Value::Null => Literal::Null,
        _ => {
            return Err(AnalysisError::UnsupportedExpression(format!(
                "Unsupported literal: {:?}",
                value
            )))
        }
    };
    Ok(literal)
}
