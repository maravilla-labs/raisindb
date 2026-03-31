//! Comparison expression analysis
//!
//! This module handles the analysis and type-checking of comparison expressions including:
//! - BETWEEN
//! - IN list
//! - LIKE
//! - ILIKE (case-insensitive LIKE)

use crate::analyzer::{
    error::AnalysisError,
    semantic::{AnalyzerContext, Result},
    typed_expr::{Expr, TypedExpr},
    types::DataType,
};
use sqlparser::ast::Expr as SqlExpr;

impl<'a> AnalyzerContext<'a> {
    /// Analyze BETWEEN expression
    pub(in crate::analyzer::semantic) fn analyze_between(
        &self,
        expr: &SqlExpr,
        negated: bool,
        low: &SqlExpr,
        high: &SqlExpr,
    ) -> Result<TypedExpr> {
        if negated {
            return Err(AnalysisError::UnsupportedExpression(
                "NOT BETWEEN not yet supported".into(),
            ));
        }
        let typed_expr = self.analyze_expr(expr)?;
        let typed_low = self.analyze_expr(low)?;
        let typed_high = self.analyze_expr(high)?;

        if typed_expr
            .data_type
            .common_type(&typed_low.data_type)
            .is_none()
        {
            return Err(AnalysisError::TypeMismatch {
                expected: typed_expr.data_type.to_string(),
                actual: typed_low.data_type.to_string(),
            });
        }

        Ok(TypedExpr::new(
            Expr::Between {
                expr: Box::new(typed_expr),
                low: Box::new(typed_low),
                high: Box::new(typed_high),
            },
            DataType::Boolean,
        ))
    }

    /// Analyze IN list expression
    pub(in crate::analyzer::semantic) fn analyze_in_list(
        &self,
        expr: &SqlExpr,
        list: &[SqlExpr],
        negated: bool,
    ) -> Result<TypedExpr> {
        let typed_expr = self.analyze_expr(expr)?;
        let typed_list: Result<Vec<_>> = list.iter().map(|e| self.analyze_expr(e)).collect();
        let typed_list = typed_list?;

        for item in &typed_list {
            if typed_expr.data_type.common_type(&item.data_type).is_none() {
                return Err(AnalysisError::TypeMismatch {
                    expected: typed_expr.data_type.to_string(),
                    actual: item.data_type.to_string(),
                });
            }
        }

        Ok(TypedExpr::new(
            Expr::InList {
                expr: Box::new(typed_expr),
                list: typed_list,
                negated,
            },
            DataType::Boolean,
        ))
    }

    /// Analyze LIKE expression
    pub(in crate::analyzer::semantic) fn analyze_like(
        &self,
        expr: &SqlExpr,
        pattern: &SqlExpr,
        negated: bool,
    ) -> Result<TypedExpr> {
        let typed_expr = self.analyze_expr(expr)?;
        let typed_pattern = self.analyze_expr(pattern)?;

        let expr_base = typed_expr.data_type.base_type();
        let pattern_base = typed_pattern.data_type.base_type();
        match (expr_base, pattern_base) {
            (DataType::Text, DataType::Text)
            | (DataType::Path, DataType::Text)
            | (DataType::Text, DataType::Path) => Ok(TypedExpr::new(
                Expr::Like {
                    expr: Box::new(typed_expr),
                    pattern: Box::new(typed_pattern),
                    negated,
                },
                DataType::Boolean,
            )),
            _ => Err(AnalysisError::TypeMismatch {
                expected: "text".to_string(),
                actual: format!(
                    "{:?} LIKE {:?}",
                    typed_expr.data_type, typed_pattern.data_type
                ),
            }),
        }
    }

    /// Analyze ILIKE expression (case-insensitive LIKE)
    pub(in crate::analyzer::semantic) fn analyze_ilike(
        &self,
        expr: &SqlExpr,
        pattern: &SqlExpr,
        negated: bool,
    ) -> Result<TypedExpr> {
        let typed_expr = self.analyze_expr(expr)?;
        let typed_pattern = self.analyze_expr(pattern)?;

        let expr_base = typed_expr.data_type.base_type();
        let pattern_base = typed_pattern.data_type.base_type();
        match (expr_base, pattern_base) {
            (DataType::Text, DataType::Text)
            | (DataType::Path, DataType::Text)
            | (DataType::Text, DataType::Path) => Ok(TypedExpr::new(
                Expr::ILike {
                    expr: Box::new(typed_expr),
                    pattern: Box::new(typed_pattern),
                    negated,
                },
                DataType::Boolean,
            )),
            _ => Err(AnalysisError::TypeMismatch {
                expected: "text".to_string(),
                actual: format!(
                    "{:?} ILIKE {:?}",
                    typed_expr.data_type, typed_pattern.data_type
                ),
            }),
        }
    }
}
