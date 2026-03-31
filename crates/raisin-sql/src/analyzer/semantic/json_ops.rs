//! JSON operator analysis
//!
//! This module handles the type-checking and analysis of PostgreSQL JSON/JSONB
//! operators like ->, ->>, @>, ?, ?|, ?&, #>, #>>, #-, @@, @?.

use super::{AnalyzerContext, Result};
use crate::analyzer::{
    error::AnalysisError,
    typed_expr::{BinaryOperator, Expr, Literal, TypedExpr},
    types::DataType,
};

impl<'a> AnalyzerContext<'a> {
    /// Analyze JSON -> operator (object extraction, returns JSONB)
    pub(super) fn analyze_json_arrow(
        &self,
        typed_left: &TypedExpr,
        typed_right: &TypedExpr,
    ) -> Result<TypedExpr> {
        // Validate: left must be JsonB, right must be Text
        if !matches!(typed_left.data_type.base_type(), DataType::JsonB) {
            return Err(AnalysisError::TypeMismatch {
                expected: "JSONB".into(),
                actual: typed_left.data_type.to_string(),
            });
        }
        if !matches!(typed_right.data_type.base_type(), DataType::Text) {
            return Err(AnalysisError::TypeMismatch {
                expected: "TEXT".into(),
                actual: typed_right.data_type.to_string(),
            });
        }
        Ok(TypedExpr::new(
            Expr::JsonExtract {
                object: Box::new(typed_left.clone()),
                key: Box::new(typed_right.clone()),
            },
            DataType::Nullable(Box::new(DataType::JsonB)),
        ))
    }

    /// Analyze JSON ->> operator (text extraction)
    pub(super) fn analyze_json_long_arrow(
        &self,
        typed_left: &TypedExpr,
        typed_right: &TypedExpr,
    ) -> Result<TypedExpr> {
        // Validate: left must be JsonB, right must be Text
        if !matches!(typed_left.data_type.base_type(), DataType::JsonB) {
            return Err(AnalysisError::TypeMismatch {
                expected: "JSONB".into(),
                actual: typed_left.data_type.to_string(),
            });
        }
        if !matches!(typed_right.data_type.base_type(), DataType::Text) {
            return Err(AnalysisError::TypeMismatch {
                expected: "TEXT".into(),
                actual: typed_right.data_type.to_string(),
            });
        }
        Ok(TypedExpr::new(
            Expr::JsonExtractText {
                object: Box::new(typed_left.clone()),
                key: Box::new(typed_right.clone()),
            },
            DataType::Nullable(Box::new(DataType::Text)),
        ))
    }

    /// Analyze JSON @> operator (containment)
    pub(super) fn analyze_json_at_arrow(
        &self,
        typed_left: &TypedExpr,
        mut typed_right: TypedExpr,
    ) -> Result<TypedExpr> {
        // Parse right side as JSON if it's a text literal
        if let Expr::Literal(Literal::Text(json_str)) = &typed_right.expr {
            match serde_json::from_str::<serde_json::Value>(json_str) {
                Ok(json_value) => {
                    typed_right =
                        TypedExpr::new(Expr::Literal(Literal::JsonB(json_value)), DataType::JsonB);
                }
                Err(e) => return Err(AnalysisError::InvalidJson(format!("Invalid JSON: {}", e))),
            }
        }

        // Validate: left must be JsonB
        if !matches!(typed_left.data_type.base_type(), DataType::JsonB) {
            return Err(AnalysisError::TypeMismatch {
                expected: "JSONB".into(),
                actual: typed_left.data_type.to_string(),
            });
        }

        Ok(TypedExpr::new(
            Expr::JsonContains {
                object: Box::new(typed_left.clone()),
                pattern: Box::new(typed_right),
            },
            DataType::Boolean,
        ))
    }

    /// Analyze JSON ? operator (key existence)
    pub(super) fn analyze_json_question(
        &self,
        typed_left: &TypedExpr,
        typed_right: &TypedExpr,
    ) -> Result<TypedExpr> {
        // Validate: left must be JsonB, right must be Text
        if !matches!(typed_left.data_type.base_type(), DataType::JsonB) {
            return Err(AnalysisError::TypeMismatch {
                expected: "JSONB".into(),
                actual: typed_left.data_type.to_string(),
            });
        }
        if !matches!(typed_right.data_type.base_type(), DataType::Text) {
            return Err(AnalysisError::TypeMismatch {
                expected: "TEXT".into(),
                actual: typed_right.data_type.to_string(),
            });
        }
        Ok(TypedExpr::new(
            Expr::JsonKeyExists {
                object: Box::new(typed_left.clone()),
                key: Box::new(typed_right.clone()),
            },
            DataType::Boolean,
        ))
    }

    /// Analyze JSON ?| operator (any key exists)
    pub(super) fn analyze_json_question_pipe(
        &self,
        typed_left: &TypedExpr,
        typed_right: &TypedExpr,
    ) -> Result<TypedExpr> {
        // Validate: left must be JsonB, right must be TEXT[] (array of text)
        if !matches!(typed_left.data_type.base_type(), DataType::JsonB) {
            return Err(AnalysisError::TypeMismatch {
                expected: "JSONB".into(),
                actual: typed_left.data_type.to_string(),
            });
        }
        // Right side should be an array of TEXT - accept either TEXT[] or an array literal
        Ok(TypedExpr::new(
            Expr::JsonAnyKeyExists {
                object: Box::new(typed_left.clone()),
                keys: Box::new(typed_right.clone()),
            },
            DataType::Boolean,
        ))
    }

    /// Analyze JSON ?& operator (all keys exist)
    pub(super) fn analyze_json_question_and(
        &self,
        typed_left: &TypedExpr,
        typed_right: &TypedExpr,
    ) -> Result<TypedExpr> {
        // Validate: left must be JsonB, right must be TEXT[] (array of text)
        if !matches!(typed_left.data_type.base_type(), DataType::JsonB) {
            return Err(AnalysisError::TypeMismatch {
                expected: "JSONB".into(),
                actual: typed_left.data_type.to_string(),
            });
        }
        // Right side should be an array of TEXT - accept either TEXT[] or an array literal
        Ok(TypedExpr::new(
            Expr::JsonAllKeyExists {
                object: Box::new(typed_left.clone()),
                keys: Box::new(typed_right.clone()),
            },
            DataType::Boolean,
        ))
    }

    /// Analyze JSON #> operator (extract at path)
    pub(super) fn analyze_json_hash_arrow(
        &self,
        typed_left: &TypedExpr,
        typed_right: &TypedExpr,
    ) -> Result<TypedExpr> {
        // Validate: left must be JsonB, right should be a path array
        if !matches!(typed_left.data_type.base_type(), DataType::JsonB) {
            return Err(AnalysisError::TypeMismatch {
                expected: "JSONB".into(),
                actual: typed_left.data_type.to_string(),
            });
        }
        // Path should be a JSONB array
        // Returns JSONB? (nullable because path might not exist)
        Ok(TypedExpr::new(
            Expr::JsonExtractPath {
                object: Box::new(typed_left.clone()),
                path: Box::new(typed_right.clone()),
            },
            DataType::Nullable(Box::new(DataType::JsonB)),
        ))
    }

    /// Analyze JSON #>> operator (extract at path as text)
    pub(super) fn analyze_json_hash_long_arrow(
        &self,
        typed_left: &TypedExpr,
        typed_right: &TypedExpr,
    ) -> Result<TypedExpr> {
        // Validate: left must be JsonB, right should be a path array
        if !matches!(typed_left.data_type.base_type(), DataType::JsonB) {
            return Err(AnalysisError::TypeMismatch {
                expected: "JSONB".into(),
                actual: typed_left.data_type.to_string(),
            });
        }
        // Path should be a JSONB array
        // Returns TEXT? (nullable because path might not exist)
        Ok(TypedExpr::new(
            Expr::JsonExtractPathText {
                object: Box::new(typed_left.clone()),
                path: Box::new(typed_right.clone()),
            },
            DataType::Nullable(Box::new(DataType::Text)),
        ))
    }

    /// Analyze JSON #- operator (remove at path)
    pub(super) fn analyze_json_hash_minus(
        &self,
        typed_left: &TypedExpr,
        typed_right: &TypedExpr,
    ) -> Result<TypedExpr> {
        // Validate: left must be JsonB, right should be a path array
        if !matches!(typed_left.data_type.base_type(), DataType::JsonB) {
            return Err(AnalysisError::TypeMismatch {
                expected: "JSONB".into(),
                actual: typed_left.data_type.to_string(),
            });
        }
        // Path should be a JSONB array
        // Returns JSONB (always returns a valid JSONB, even if path doesn't exist)
        Ok(TypedExpr::new(
            Expr::JsonRemoveAtPath {
                object: Box::new(typed_left.clone()),
                path: Box::new(typed_right.clone()),
            },
            DataType::JsonB,
        ))
    }

    /// Analyze JSON @@ operator (path match)
    pub(super) fn analyze_json_at_at(
        &self,
        typed_left: &TypedExpr,
        typed_right: &TypedExpr,
    ) -> Result<TypedExpr> {
        // Validate: left must be JsonB, right should be JSONPath expression (TEXT)
        if !matches!(typed_left.data_type.base_type(), DataType::JsonB) {
            return Err(AnalysisError::TypeMismatch {
                expected: "JSONB".into(),
                actual: typed_left.data_type.to_string(),
            });
        }
        // Path should be TEXT (JSONPath expression)
        // Returns BOOLEAN
        Ok(TypedExpr::new(
            Expr::JsonPathMatch {
                object: Box::new(typed_left.clone()),
                path: Box::new(typed_right.clone()),
            },
            DataType::Boolean,
        ))
    }

    /// Analyze JSON @? operator (path exists)
    pub(super) fn analyze_json_at_question(
        &self,
        typed_left: &TypedExpr,
        typed_right: &TypedExpr,
    ) -> Result<TypedExpr> {
        // Validate: left must be JsonB, right should be JSONPath expression (TEXT)
        if !matches!(typed_left.data_type.base_type(), DataType::JsonB) {
            return Err(AnalysisError::TypeMismatch {
                expected: "JSONB".into(),
                actual: typed_left.data_type.to_string(),
            });
        }
        // Path should be TEXT (JSONPath expression)
        // Returns BOOLEAN
        Ok(TypedExpr::new(
            Expr::JsonPathExists {
                object: Box::new(typed_left.clone()),
                path: Box::new(typed_right.clone()),
            },
            DataType::Boolean,
        ))
    }

    /// Analyze || operator (string concatenation or JSONB merge)
    pub(super) fn analyze_string_concat(
        &self,
        typed_left: &TypedExpr,
        typed_right: &TypedExpr,
    ) -> Result<TypedExpr> {
        let left_is_jsonb = matches!(typed_left.data_type.base_type(), DataType::JsonB);
        let right_is_jsonb = matches!(typed_right.data_type.base_type(), DataType::JsonB);

        // If left operand is JSONB, try to parse right operand as JSON
        if left_is_jsonb {
            // Try to parse right side as JSON if it's a text literal
            let typed_right_final = if let Expr::Literal(Literal::Text(json_str)) =
                &typed_right.expr
            {
                match serde_json::from_str::<serde_json::Value>(json_str) {
                    Ok(json_value) => {
                        TypedExpr::new(Expr::Literal(Literal::JsonB(json_value)), DataType::JsonB)
                    }
                    Err(e) => {
                        return Err(AnalysisError::InvalidJson(format!("Invalid JSON: {}", e)))
                    }
                }
            } else {
                typed_right.clone()
            };

            // Now check if both are JSONB
            if matches!(typed_right_final.data_type.base_type(), DataType::JsonB) {
                return Ok(TypedExpr::new(
                    Expr::BinaryOp {
                        left: Box::new(typed_left.clone()),
                        op: BinaryOperator::JsonConcat,
                        right: Box::new(typed_right_final),
                    },
                    DataType::JsonB,
                ));
            }
        }

        // Check if right operand is JSONB and try to parse left as JSON
        // Only check this if left is NOT JSONB (to avoid double processing)
        if right_is_jsonb && !left_is_jsonb {
            if let Expr::Literal(Literal::Text(json_str)) = &typed_left.expr {
                match serde_json::from_str::<serde_json::Value>(json_str) {
                    Ok(json_value) => {
                        let typed_left_final = TypedExpr::new(
                            Expr::Literal(Literal::JsonB(json_value)),
                            DataType::JsonB,
                        );
                        return Ok(TypedExpr::new(
                            Expr::BinaryOp {
                                left: Box::new(typed_left_final),
                                op: BinaryOperator::JsonConcat,
                                right: Box::new(typed_right.clone()),
                            },
                            DataType::JsonB,
                        ));
                    }
                    Err(e) => {
                        return Err(AnalysisError::InvalidJson(format!("Invalid JSON: {}", e)))
                    }
                }
            }
        }

        // String concatenation: Text || Text → Text
        // Also works with Path types (coerced to Text)
        if matches!(
            typed_left.data_type.base_type(),
            DataType::Text | DataType::Path
        ) || matches!(
            typed_right.data_type.base_type(),
            DataType::Text | DataType::Path
        ) {
            return Ok(TypedExpr::new(
                Expr::BinaryOp {
                    left: Box::new(typed_left.clone()),
                    op: BinaryOperator::StringConcat,
                    right: Box::new(typed_right.clone()),
                },
                DataType::Text,
            ));
        }

        // Neither operand is JSONB or Text - unsupported
        Err(AnalysisError::UnsupportedExpression(
            "Concatenation (||) only supported for JSONB or Text types.".into(),
        ))
    }
}
