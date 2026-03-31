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

        Ok(TypedExpr::literal(literal))
    }
}
