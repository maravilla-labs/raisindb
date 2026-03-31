//! Core function analysis and special function handling
//!
//! Contains the main `analyze_function` entry point plus special-case handling
//! for COALESCE, TO_JSON/TO_JSONB, and JSON_GET functions.

use super::super::{AnalyzerContext, Result};
use crate::analyzer::{
    error::AnalysisError,
    functions::{FunctionCategory, FunctionSignature},
    typed_expr::{Expr, Literal, TypedExpr},
    types::DataType,
};
use sqlparser::ast::{Function as SqlFunc, FunctionArg, FunctionArgExpr, FunctionArguments};

impl<'a> AnalyzerContext<'a> {
    /// Try to handle TO_JSON/TO_JSONB with a table reference argument.
    /// Returns Some(TypedExpr) if the function is TO_JSON(table_alias), None otherwise.
    pub(in crate::analyzer::semantic) fn try_handle_to_json_table_ref(
        &self,
        func_name: &str,
        func: &SqlFunc,
    ) -> Option<TypedExpr> {
        // Only handle TO_JSON/TO_JSONB
        if func_name != "TO_JSON" && func_name != "TO_JSONB" {
            return None;
        }

        // Must have exactly one argument in a list
        let FunctionArguments::List(arg_list) = &func.args else {
            return None;
        };
        if arg_list.args.len() != 1 {
            return None;
        }

        // Argument must be an unnamed identifier expression
        let FunctionArg::Unnamed(FunctionArgExpr::Expr(sqlparser::ast::Expr::Identifier(ident))) =
            &arg_list.args[0]
        else {
            return None;
        };

        let identifier = &ident.value;

        // Check if this identifier is a table alias (not a column)
        self.current_tables
            .iter()
            .find(|t| t.name() == identifier)?;

        // This is a table reference - create special expression for physical executor
        let arg_expr = TypedExpr::new(
            Expr::Column {
                table: identifier.clone(),
                column: identifier.clone(),
            },
            DataType::Unknown,
        );

        Some(TypedExpr::new(
            Expr::Function {
                name: func_name.to_string(),
                args: vec![arg_expr],
                signature: FunctionSignature {
                    name: func_name.to_string(),
                    params: vec![DataType::Unknown],
                    return_type: DataType::JsonB,
                    is_deterministic: true,
                    category: FunctionCategory::Scalar,
                },
                filter: None,
            },
            DataType::JsonB,
        ))
    }

    /// Analyze a function call
    pub(in crate::analyzer::semantic) fn analyze_function(
        &self,
        func: &SqlFunc,
    ) -> Result<TypedExpr> {
        let func_name = func.name.to_string().to_uppercase();

        // Special early handling for TO_JSON/TO_JSONB - check for table references before analyzing args
        if let Some(result) = self.try_handle_to_json_table_ref(&func_name, func) {
            return Ok(result);
        }

        // Extract arguments
        let args: Result<Vec<TypedExpr>> = match &func.args {
            FunctionArguments::None => Ok(vec![]),
            FunctionArguments::List(arg_list) => arg_list
                .args
                .iter()
                .map(|arg| match arg {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => self.analyze_expr(expr),
                    FunctionArg::Named {
                        arg: FunctionArgExpr::Expr(expr),
                        ..
                    } => self.analyze_expr(expr),
                    FunctionArg::Unnamed(FunctionArgExpr::Wildcard) => {
                        // For COUNT(*)
                        Ok(TypedExpr::new(
                            Expr::Literal(Literal::Int(1)),
                            DataType::Unknown,
                        ))
                    }
                    _ => Err(AnalysisError::UnsupportedExpression(
                        "Unsupported function argument".into(),
                    )),
                })
                .collect(),
            FunctionArguments::Subquery(_) => Err(AnalysisError::SubqueriesNotSupported),
        };

        let args = args?;
        let arg_types: Vec<DataType> = args.iter().map(|a| a.data_type.clone()).collect();

        // Check if this is a window function (has OVER clause)
        if let Some(over) = &func.over {
            return self.analyze_window_function(&func_name, args, over);
        }

        // Special handling for COALESCE - variadic function with polymorphic return type
        if func_name == "COALESCE" {
            return self.analyze_coalesce(args, arg_types);
        }

        // Special handling for TO_JSON/TO_JSONB function
        if func_name == "TO_JSON" || func_name == "TO_JSONB" {
            return self.analyze_to_json(&func_name, &args);
        }

        // Special handling for JSON_GET function
        if func_name == "JSON_GET" {
            return self.analyze_json_get(&args);
        }

        // Resolve function signature
        let signature = self
            .functions
            .resolve(&func_name, &arg_types)
            .ok_or_else(|| AnalysisError::FunctionNotFound {
                name: func_name.clone(),
                args: arg_types
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            })?;

        // Coerce arguments to match signature parameter types
        let mut coerced_args = Vec::new();
        for (arg, param_type) in args.iter().zip(&signature.params) {
            let coerced = self.coerce_if_needed(arg.clone(), param_type)?;
            coerced_args.push(coerced);
        }
        let args = coerced_args;

        // Apply constant folding for deterministic functions
        if signature.is_deterministic && args.iter().all(|a| matches!(a.expr, Expr::Literal(_))) {
            if let Some(folded) = self.try_constant_fold(&func_name, &args)? {
                return Ok(folded);
            }
        }

        // Analyze FILTER clause if present (for aggregate functions)
        let filter = if let Some(filter_expr) = &func.filter {
            let analyzed_filter = self.analyze_expr(filter_expr)?;
            // Validate filter is a boolean expression
            if !matches!(analyzed_filter.data_type.base_type(), DataType::Boolean) {
                return Err(AnalysisError::TypeMismatch {
                    expected: "BOOLEAN".into(),
                    actual: analyzed_filter.data_type.to_string(),
                });
            }
            Some(Box::new(analyzed_filter))
        } else {
            None
        };

        // Special handling for EMBEDDING function - resolve return type based on catalog config
        let return_type = if func_name == "EMBEDDING" {
            let dimensions = self.catalog.embedding_dimensions().ok_or_else(|| {
                AnalysisError::UnsupportedExpression(
                    "EMBEDDING() function requires embeddings to be enabled. \
                     Configure embedding provider via tenant settings."
                        .into(),
                )
            })?;
            DataType::Vector(dimensions)
        } else {
            signature.return_type.clone()
        };

        Ok(TypedExpr::new(
            Expr::Function {
                name: func_name,
                args,
                signature: signature.clone(),
                filter,
            },
            return_type,
        ))
    }

    /// Analyze COALESCE function
    fn analyze_coalesce(
        &self,
        args: Vec<TypedExpr>,
        arg_types: Vec<DataType>,
    ) -> Result<TypedExpr> {
        if args.is_empty() {
            return Err(AnalysisError::FunctionNotFound {
                name: "COALESCE".to_string(),
                args: "COALESCE requires at least 1 argument".to_string(),
            });
        }

        // Determine base return type - common type of all arguments
        let mut base_type = args[0].data_type.base_type().clone();
        for arg in &args[1..] {
            let arg_base = arg.data_type.base_type();
            if let Some(common) = base_type.common_type(arg_base) {
                base_type = common.base_type().clone();
            } else {
                return Err(AnalysisError::TypeMismatch {
                    expected: base_type.to_string(),
                    actual: arg.data_type.to_string(),
                });
            }
        }

        // COALESCE returns non-nullable if the LAST argument is non-nullable
        let last_arg_nullable = args
            .last()
            .map(|a| a.data_type.is_nullable())
            .unwrap_or(true);

        let return_type = if last_arg_nullable {
            base_type.as_nullable()
        } else {
            base_type
        };

        Ok(TypedExpr::new(
            Expr::Function {
                name: "COALESCE".to_string(),
                args,
                signature: FunctionSignature {
                    name: "COALESCE".into(),
                    params: arg_types,
                    return_type: return_type.clone(),
                    is_deterministic: true,
                    category: FunctionCategory::Scalar,
                },
                filter: None,
            },
            return_type,
        ))
    }

    /// Analyze TO_JSON/TO_JSONB function (column to JSON conversion)
    fn analyze_to_json(&self, func_name: &str, args: &[TypedExpr]) -> Result<TypedExpr> {
        if args.len() != 1 {
            return Err(AnalysisError::FunctionNotFound {
                name: func_name.to_string(),
                args: format!(
                    "{}(expr) requires exactly 1 argument, got {}",
                    func_name,
                    args.len()
                ),
            });
        }

        let arg_expr = &args[0];

        // At this point, if it was a table reference, it would have been handled early
        // So this must be a regular column expression - just cast to JSONB
        Ok(TypedExpr::new(
            Expr::Cast {
                expr: Box::new(arg_expr.clone()),
                target_type: DataType::JsonB,
            },
            DataType::JsonB,
        ))
    }

    /// Analyze JSON_GET function - JSON path extraction with table-qualified columns
    fn analyze_json_get(&self, args: &[TypedExpr]) -> Result<TypedExpr> {
        if args.len() != 2 {
            return Err(AnalysisError::FunctionNotFound {
                name: "JSON_GET".to_string(),
                args: format!(
                    "JSON_GET(column, path) requires exactly 2 arguments, got {}",
                    args.len()
                ),
            });
        }

        let column_expr = args[0].clone();
        let path_expr = &args[1];

        // Second argument must be a string literal representing the JSON path
        let json_path = match &path_expr.expr {
            Expr::Literal(Literal::Text(s)) => s.clone(),
            _ => {
                return Err(AnalysisError::TypeMismatch {
                    expected: "string literal for JSON path".to_string(),
                    actual: format!("{:?}", path_expr.data_type),
                });
            }
        };

        // Split JSON path by dots to create array elements
        let path_parts: Vec<String> = json_path
            .split('.')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        if path_parts.is_empty() {
            return Err(AnalysisError::UnsupportedExpression(
                "JSON_GET(column, path): JSON path cannot be empty".into(),
            ));
        }

        // Build the JSON path array as JSONB literal: ARRAY['part1', 'part2', ...]
        let json_array_values: Vec<serde_json::Value> = path_parts
            .iter()
            .map(|part| serde_json::Value::String(part.clone()))
            .collect();

        let path_array = TypedExpr::new(
            Expr::Literal(Literal::JsonB(serde_json::Value::Array(json_array_values))),
            DataType::JsonB,
        );

        // Cast column to JSONB
        let jsonb_column = TypedExpr::new(
            Expr::Cast {
                expr: Box::new(column_expr),
                target_type: DataType::JsonB,
            },
            DataType::JsonB,
        );

        // Create JsonExtractPath operator: column_as_jsonb #> path_array
        Ok(TypedExpr::new(
            Expr::JsonExtractPath {
                object: Box::new(jsonb_column),
                path: Box::new(path_array),
            },
            DataType::Nullable(Box::new(DataType::JsonB)),
        ))
    }
}
