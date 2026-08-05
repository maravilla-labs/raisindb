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

        // `__revision = '<timestamp>-<counter>'` — a point-in-time read pinned to
        // a FULL HLC. The pseudo-column is declared BigInt (so `__revision = 342`
        // keeps working), which would make the string form an InvalidBinaryOp
        // before `extract_revision_predicate` ever sees it — but a bare timestamp
        // cannot express the HLC counter, so same-millisecond revisions are
        // indistinguishable. Type the comparison here and let the extractor parse
        // the literal; an unparseable revision is an error NOW rather than a
        // predicate that silently survives into execution.
        if let Some(revision_cmp) =
            self.analyze_revision_comparison(&typed_left, op, &typed_right)?
        {
            return Ok(revision_cmp);
        }

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

    /// Type a `__revision = '<timestamp>-<counter>'` comparison as Boolean.
    ///
    /// Returns `Ok(None)` when this isn't a `__revision`-against-string-literal
    /// comparison, so the caller falls through to the normal operator rules
    /// (`__revision = 342` still types via the BigInt column). The literal is
    /// left untouched in the expression — `extract_revision_predicate` parses it
    /// into the query's `max_revision` and drops the predicate; validating it
    /// here means a malformed revision fails analysis instead of reaching
    /// execution as an unsatisfiable BigInt-vs-Text comparison.
    fn analyze_revision_comparison(
        &self,
        left: &TypedExpr,
        op: &SqlBinaryOp,
        right: &TypedExpr,
    ) -> Result<Option<TypedExpr>> {
        if !matches!(op, SqlBinaryOp::Eq) {
            return Ok(None);
        }
        let is_revision_column = |e: &TypedExpr| matches!(&e.expr, Expr::Column { column, .. } if column == "__revision");
        let string_literal = |e: &TypedExpr| match &e.expr {
            Expr::Literal(Literal::Text(s)) => Some(s.clone()),
            _ => None,
        };

        let literal = if is_revision_column(left) {
            string_literal(right)
        } else if is_revision_column(right) {
            string_literal(left)
        } else {
            None
        };
        let Some(literal) = literal else {
            return Ok(None);
        };

        if literal.parse::<raisin_hlc::HLC>().is_err() {
            return Err(AnalysisError::TypeMismatch {
                expected: "__revision as 'timestamp-counter' (HLC) or a timestamp integer".into(),
                actual: format!("'{}'", literal),
            });
        }

        Ok(Some(TypedExpr::new(
            Expr::BinaryOp {
                left: Box::new(left.clone()),
                op: BinaryOperator::Eq,
                right: Box::new(right.clone()),
            },
            DataType::Boolean,
        )))
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
