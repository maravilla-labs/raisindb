//! CAST expression and type conversion analysis
//!
//! This module handles the analysis and type-checking of:
//! - CAST expressions
//! - SQL type to internal type conversion

use crate::analyzer::{
    error::AnalysisError,
    semantic::{AnalyzerContext, Result},
    typed_expr::{Expr, TypedExpr},
    types::DataType,
};
use sqlparser::ast::{DataType as SqlDataType, Expr as SqlExpr};

impl<'a> AnalyzerContext<'a> {
    /// Analyze CAST expression
    pub(in crate::analyzer::semantic) fn analyze_cast(
        &self,
        expr: &SqlExpr,
        target_type: &SqlDataType,
    ) -> Result<TypedExpr> {
        let typed_expr = self.analyze_expr(expr)?;
        let target = self.convert_sql_type(target_type)?;

        if typed_expr.data_type.can_cast_to(&target) {
            return Ok(TypedExpr::new(
                Expr::Cast {
                    expr: Box::new(typed_expr),
                    target_type: target.clone(),
                },
                target,
            ));
        }

        // Check if intermediate cast through TEXT is possible
        if let Some(intermediate) = typed_expr.data_type.get_intermediate_cast_type(&target) {
            let source_is_nullable = typed_expr.data_type.is_nullable();
            let intermediate_type = if source_is_nullable {
                intermediate.clone().as_nullable()
            } else {
                intermediate.clone()
            };

            let intermediate_expr = TypedExpr::new(
                Expr::Cast {
                    expr: Box::new(typed_expr),
                    target_type: intermediate,
                },
                intermediate_type,
            );

            return Ok(TypedExpr::new(
                Expr::Cast {
                    expr: Box::new(intermediate_expr),
                    target_type: target.clone(),
                },
                target,
            ));
        }

        Err(AnalysisError::InvalidCoercion {
            from: typed_expr.data_type.to_string(),
            to: target.to_string(),
        })
    }

    /// Convert SQL data type to internal representation
    pub(in crate::analyzer::semantic) fn convert_sql_type(
        &self,
        sql_type: &SqlDataType,
    ) -> Result<DataType> {
        match sql_type {
            SqlDataType::Int(_) | SqlDataType::Integer(_) => Ok(DataType::Int),
            SqlDataType::BigInt(_) => Ok(DataType::BigInt),
            SqlDataType::Double(_) | SqlDataType::DoublePrecision => Ok(DataType::Double),
            SqlDataType::Boolean => Ok(DataType::Boolean),
            SqlDataType::Text | SqlDataType::Varchar(_) | SqlDataType::String(_) => {
                Ok(DataType::Text)
            }
            SqlDataType::Timestamp(_, _) => Ok(DataType::TimestampTz),
            SqlDataType::JSON => Ok(DataType::JsonB),
            SqlDataType::JSONB => Ok(DataType::JsonB),
            SqlDataType::Custom(name, _) => {
                let type_name = name
                    .0
                    .iter()
                    .filter_map(|part| part.as_ident().map(|i| i.value.as_str()))
                    .collect::<Vec<_>>()
                    .join(".");
                match type_name.to_uppercase().as_str() {
                    "PATH" => Ok(DataType::Path),
                    "JSONB" => Ok(DataType::JsonB),
                    "UUID" => Ok(DataType::Uuid),
                    "GEOMETRY" => Ok(DataType::Geometry),
                    "TSVECTOR" => Ok(DataType::TSVector),
                    "TSQUERY" => Ok(DataType::TSQuery),
                    "INTERVAL" => Ok(DataType::Interval),
                    _ => Err(AnalysisError::UnsupportedExpression(format!(
                        "Unsupported type: {}",
                        type_name
                    ))),
                }
            }
            _ => Err(AnalysisError::UnsupportedExpression(format!(
                "Unsupported SQL type: {:?}",
                sql_type
            ))),
        }
    }
}
