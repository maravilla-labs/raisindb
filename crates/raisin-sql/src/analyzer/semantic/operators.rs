//! Binary and unary operator analysis
//!
//! This module handles the type-checking and conversion of SQL binary
//! and unary operators to their internal representation.

use super::{AnalyzerContext, Result};
use crate::analyzer::{
    error::AnalysisError,
    typed_expr::{BinaryOperator, Expr, Literal, TypedExpr, UnaryOperator},
    types::DataType,
};
use sqlparser::ast::{BinaryOperator as SqlBinaryOp, UnaryOperator as SqlUnaryOp};

impl<'a> AnalyzerContext<'a> {
    /// Analyze a binary operation
    pub(super) fn analyze_binary_op(
        &self,
        left: &sqlparser::ast::Expr,
        op: &SqlBinaryOp,
        right: &sqlparser::ast::Expr,
    ) -> Result<TypedExpr> {
        let typed_left = self.analyze_expr(left)?;
        let typed_right = self.analyze_expr(right)?;

        // Special handling for JSON operators
        match op {
            SqlBinaryOp::Arrow => {
                return self.analyze_json_arrow(&typed_left, &typed_right);
            }
            SqlBinaryOp::LongArrow => {
                return self.analyze_json_long_arrow(&typed_left, &typed_right);
            }
            SqlBinaryOp::AtArrow => {
                return self.analyze_json_at_arrow(&typed_left, typed_right);
            }
            SqlBinaryOp::Question => {
                return self.analyze_json_question(&typed_left, &typed_right);
            }
            SqlBinaryOp::QuestionPipe => {
                return self.analyze_json_question_pipe(&typed_left, &typed_right);
            }
            SqlBinaryOp::QuestionAnd => {
                return self.analyze_json_question_and(&typed_left, &typed_right);
            }
            SqlBinaryOp::HashArrow => {
                return self.analyze_json_hash_arrow(&typed_left, &typed_right);
            }
            SqlBinaryOp::HashLongArrow => {
                return self.analyze_json_hash_long_arrow(&typed_left, &typed_right);
            }
            SqlBinaryOp::HashMinus => {
                return self.analyze_json_hash_minus(&typed_left, &typed_right);
            }
            SqlBinaryOp::AtAt => {
                return self.analyze_json_at_at(&typed_left, &typed_right);
            }
            SqlBinaryOp::AtQuestion => {
                return self.analyze_json_at_question(&typed_left, &typed_right);
            }
            SqlBinaryOp::Minus => {
                // - operator: Subtraction or JSONB key/element removal
                if matches!(typed_left.data_type.base_type(), DataType::JsonB) {
                    return Ok(TypedExpr::new(
                        Expr::JsonRemove {
                            object: Box::new(typed_left),
                            key: Box::new(typed_right),
                        },
                        DataType::JsonB,
                    ));
                }
                // Fall through to numeric subtraction handling
            }
            SqlBinaryOp::StringConcat => {
                return self.analyze_string_concat(&typed_left, &typed_right);
            }
            _ => {}
        }

        let binary_op = self.convert_binary_op(op)?;
        let result_type = binary_op
            .result_type(&typed_left.data_type, &typed_right.data_type)
            .ok_or_else(|| AnalysisError::InvalidBinaryOp {
                left: typed_left.data_type.to_string(),
                op: format!("{:?}", op),
                right: typed_right.data_type.to_string(),
            })?;

        Ok(TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(typed_left),
                op: binary_op,
                right: Box::new(typed_right),
            },
            result_type,
        ))
    }

    /// Convert SQL binary operator to internal representation
    pub(super) fn convert_binary_op(&self, op: &SqlBinaryOp) -> Result<BinaryOperator> {
        match op {
            SqlBinaryOp::Plus => Ok(BinaryOperator::Add),
            SqlBinaryOp::Minus => Ok(BinaryOperator::Subtract),
            SqlBinaryOp::Multiply => Ok(BinaryOperator::Multiply),
            SqlBinaryOp::Divide => Ok(BinaryOperator::Divide),
            SqlBinaryOp::Modulo => Ok(BinaryOperator::Modulo),
            SqlBinaryOp::Eq => Ok(BinaryOperator::Eq),
            SqlBinaryOp::NotEq => Ok(BinaryOperator::NotEq),
            SqlBinaryOp::Lt => Ok(BinaryOperator::Lt),
            SqlBinaryOp::LtEq => Ok(BinaryOperator::LtEq),
            SqlBinaryOp::Gt => Ok(BinaryOperator::Gt),
            SqlBinaryOp::GtEq => Ok(BinaryOperator::GtEq),
            SqlBinaryOp::And => Ok(BinaryOperator::And),
            SqlBinaryOp::Or => Ok(BinaryOperator::Or),
            // JSON operators
            SqlBinaryOp::Arrow => Ok(BinaryOperator::JsonExtract),
            SqlBinaryOp::LongArrow => Ok(BinaryOperator::JsonExtract),
            SqlBinaryOp::AtArrow => Ok(BinaryOperator::JsonContains),
            SqlBinaryOp::ArrowAt => Ok(BinaryOperator::JsonContains),
            // Vector distance operators (pgvector-compatible)
            SqlBinaryOp::LtDashGt => Ok(BinaryOperator::VectorL2Distance),
            SqlBinaryOp::Spaceship => Ok(BinaryOperator::VectorCosineDistance),
            _ => {
                // Check if this is a custom PostgreSQL operator string
                let op_str = format!("{:?}", op);
                if op_str.contains("<->") || op_str.contains("\"<->\"") {
                    Ok(BinaryOperator::VectorL2Distance)
                } else if op_str.contains("<=>") || op_str.contains("\"<=>\"") {
                    Ok(BinaryOperator::VectorCosineDistance)
                } else if op_str.contains("<#>") || op_str.contains("\"<#>\"") {
                    Ok(BinaryOperator::VectorInnerProduct)
                } else {
                    Err(AnalysisError::UnsupportedExpression(format!(
                        "Unsupported binary operator: {:?}",
                        op
                    )))
                }
            }
        }
    }

    /// Analyze unary operation
    pub(super) fn analyze_unary_op(
        &self,
        op: &SqlUnaryOp,
        expr: &sqlparser::ast::Expr,
    ) -> Result<TypedExpr> {
        let typed_expr = self.analyze_expr(expr)?;

        let unary_op = match op {
            SqlUnaryOp::Not => UnaryOperator::Not,
            SqlUnaryOp::Minus => UnaryOperator::Negate,
            _ => {
                return Err(AnalysisError::UnsupportedExpression(format!(
                    "Unsupported unary operator: {:?}",
                    op
                )))
            }
        };

        let result_type = unary_op.result_type(&typed_expr.data_type).ok_or_else(|| {
            AnalysisError::InvalidUnaryOp {
                op: format!("{:?}", op),
                operand: typed_expr.data_type.to_string(),
            }
        })?;

        Ok(TypedExpr::new(
            Expr::UnaryOp {
                op: unary_op,
                expr: Box::new(typed_expr),
            },
            result_type,
        ))
    }

    /// Coerce an expression to a target type if needed
    pub(super) fn coerce_if_needed(&self, expr: TypedExpr, target: &DataType) -> Result<TypedExpr> {
        // If we have Text literal and need Path
        if let Expr::Literal(Literal::Text(s)) = &expr.expr {
            if matches!(target, DataType::Path) {
                // Validate path syntax
                if !s.starts_with('/') {
                    return Err(AnalysisError::InvalidPath(format!(
                        "Path must start with '/': {}",
                        s
                    )));
                }
                // Return coerced expression
                return Ok(TypedExpr::new(
                    Expr::Literal(Literal::Path(s.clone())),
                    DataType::Path,
                ));
            }
        }

        // Check if types are compatible
        if !expr.data_type.can_coerce_to(target) {
            return Err(AnalysisError::InvalidCoercion {
                from: expr.data_type.to_string(),
                to: target.to_string(),
            });
        }

        Ok(expr)
    }
}
