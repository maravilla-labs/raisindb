//! CASE expression analysis
//!
//! This module handles the analysis and type-checking of SQL CASE expressions including:
//! - Simple CASE: CASE x WHEN 1 THEN 'a' WHEN 2 THEN 'b' END
//! - Searched CASE: CASE WHEN x > 0 THEN 'positive' ELSE 'non-positive' END

use crate::analyzer::{
    error::AnalysisError,
    semantic::{AnalyzerContext, Result},
    typed_expr::{BinaryOperator, Expr, Literal, TypedExpr},
    types::DataType,
};
use sqlparser::ast::Expr as SqlExpr;

impl<'a> AnalyzerContext<'a> {
    /// Analyze CASE expression
    pub(in crate::analyzer::semantic) fn analyze_case(
        &self,
        operand: &Option<Box<SqlExpr>>,
        conditions: &[sqlparser::ast::CaseWhen],
        else_result: &Option<Box<SqlExpr>>,
    ) -> Result<TypedExpr> {
        if conditions.is_empty() {
            return Err(AnalysisError::UnsupportedExpression(
                "CASE expression: must have at least one WHEN clause".into(),
            ));
        }

        // For simple CASE (operand is Some), convert to searched form
        let (typed_conditions, typed_results) = if let Some(operand_expr) = operand {
            let typed_operand = self.analyze_expr(operand_expr)?;

            let mut typed_conds = Vec::new();
            let mut typed_ress = Vec::new();

            for case_when in conditions.iter() {
                let typed_when_value = self.analyze_expr(&case_when.condition)?;

                if typed_operand
                    .data_type
                    .common_type(&typed_when_value.data_type)
                    .is_none()
                {
                    return Err(AnalysisError::TypeMismatch {
                        expected: typed_operand.data_type.to_string(),
                        actual: typed_when_value.data_type.to_string(),
                    });
                }

                let condition = TypedExpr::new(
                    Expr::BinaryOp {
                        left: Box::new(typed_operand.clone()),
                        op: BinaryOperator::Eq,
                        right: Box::new(typed_when_value),
                    },
                    DataType::Boolean,
                );

                typed_conds.push(condition);
                typed_ress.push(self.analyze_expr(&case_when.result)?);
            }

            (typed_conds, typed_ress)
        } else {
            let mut typed_conds = Vec::new();
            let mut typed_ress = Vec::new();

            for case_when in conditions.iter() {
                let typed_condition = self.analyze_expr(&case_when.condition)?;
                if !matches!(typed_condition.data_type.base_type(), DataType::Boolean) {
                    return Err(AnalysisError::TypeMismatch {
                        expected: "BOOLEAN".into(),
                        actual: typed_condition.data_type.to_string(),
                    });
                }
                typed_conds.push(typed_condition);
                typed_ress.push(self.analyze_expr(&case_when.result)?);
            }

            (typed_conds, typed_ress)
        };

        let typed_else = if let Some(else_expr) = else_result {
            Some(Box::new(self.analyze_expr(else_expr)?))
        } else {
            None
        };

        // Determine result type
        let mut result_type = typed_results[0].data_type.clone();

        for typed_result in &typed_results[1..] {
            if let Some(common) = result_type.common_type(&typed_result.data_type) {
                result_type = common;
            } else {
                return Err(AnalysisError::TypeMismatch {
                    expected: result_type.to_string(),
                    actual: typed_result.data_type.to_string(),
                });
            }
        }

        if let Some(else_expr) = &typed_else {
            if matches!(else_expr.expr, Expr::Literal(Literal::Null)) {
                result_type = DataType::Nullable(Box::new(result_type));
            } else if let Some(common) = result_type.common_type(&else_expr.data_type) {
                result_type = common;
            } else {
                return Err(AnalysisError::TypeMismatch {
                    expected: result_type.to_string(),
                    actual: else_expr.data_type.to_string(),
                });
            }
        } else {
            result_type = DataType::Nullable(Box::new(result_type));
        }

        let conditions_with_results: Vec<(TypedExpr, TypedExpr)> =
            typed_conditions.into_iter().zip(typed_results).collect();

        Ok(TypedExpr::new(
            Expr::Case {
                conditions: conditions_with_results,
                else_expr: typed_else,
            },
            result_type,
        ))
    }
}
