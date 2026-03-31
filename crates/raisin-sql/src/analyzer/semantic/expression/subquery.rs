//! Subquery expression analysis
//!
//! This module handles the analysis and type-checking of subquery expressions including:
//! - IN subquery: expr IN (SELECT ...)

use crate::analyzer::{
    error::AnalysisError,
    semantic::{AnalyzerContext, Result},
    typed_expr::{Expr, TypedExpr},
    types::DataType,
};
use sqlparser::ast::Expr as SqlExpr;

impl<'a> AnalyzerContext<'a> {
    /// Analyze IN subquery expression
    pub(in crate::analyzer::semantic) fn analyze_in_subquery(
        &self,
        expr: &SqlExpr,
        subquery: &sqlparser::ast::Query,
        negated: bool,
    ) -> Result<TypedExpr> {
        let typed_expr = self.analyze_expr(expr)?;

        // Create a new analyzer context for the subquery that inherits CTE definitions
        let mut subquery_ctx = AnalyzerContext {
            catalog: self.catalog,
            functions: self.functions,
            current_tables: Vec::new(),
            cte_catalog: self.cte_catalog.clone(),
            is_upsert: false,
        };

        let mut analyzed_subquery = subquery_ctx.analyze_query(subquery)?;

        // Copy outer CTEs to the subquery
        for (cte_name, cte_def) in &self.cte_catalog {
            if !analyzed_subquery
                .ctes
                .iter()
                .any(|(name, _)| name == cte_name)
            {
                analyzed_subquery
                    .ctes
                    .push((cte_name.clone(), cte_def.query.clone()));
            }
        }

        // Validate subquery returns exactly one column
        if analyzed_subquery.projection.len() != 1 {
            return Err(AnalysisError::UnsupportedExpression(format!(
                "IN subquery must return exactly one column, got {}",
                analyzed_subquery.projection.len()
            )));
        }

        let subquery_type = analyzed_subquery.projection[0].0.data_type.clone();

        if typed_expr.data_type.common_type(&subquery_type).is_none() {
            return Err(AnalysisError::TypeMismatch {
                expected: typed_expr.data_type.to_string(),
                actual: subquery_type.to_string(),
            });
        }

        Ok(TypedExpr::new(
            Expr::InSubquery {
                expr: Box::new(typed_expr),
                subquery: Box::new(analyzed_subquery),
                subquery_type,
                negated,
            },
            DataType::Boolean,
        ))
    }
}
