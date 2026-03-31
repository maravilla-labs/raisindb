//! INTERVAL expression analysis
//!
//! This module handles the analysis and type-checking of SQL INTERVAL expressions.

use crate::analyzer::{
    error::AnalysisError,
    semantic::{parse_interval_string, AnalyzerContext, Result},
    typed_expr::{Expr, Literal, TypedExpr},
};

impl<'a> AnalyzerContext<'a> {
    /// Analyze INTERVAL expression
    pub(in crate::analyzer::semantic) fn analyze_interval(
        &self,
        interval: &sqlparser::ast::Interval,
    ) -> Result<TypedExpr> {
        let value_expr = self.analyze_expr(&interval.value)?;

        let interval_str = if let Expr::Literal(Literal::Text(s)) = &value_expr.expr {
            s.clone()
        } else {
            return Err(AnalysisError::UnsupportedExpression(
                "INTERVAL value must be a string literal".into(),
            ));
        };

        let duration = parse_interval_string(&interval_str)?;
        Ok(TypedExpr::literal(Literal::Interval(duration)))
    }
}
