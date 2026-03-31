//! Dollar-dot JSON path syntax analysis
//!
//! This module handles the analysis and expansion of $.column.path syntax into
//! proper JSON extraction expressions (column::JSONB #> ARRAY['path', ...]).

use crate::analyzer::{
    error::AnalysisError,
    semantic::{AnalyzerContext, Result},
    typed_expr::{Expr, Literal, TypedExpr},
    types::DataType,
};
use sqlparser::ast::{Expr as SqlExpr, FunctionArguments, Ident, Value};

impl<'a> AnalyzerContext<'a> {
    /// Analyze compound field access ($.column.path syntax)
    pub(in crate::analyzer::semantic) fn analyze_compound_field_access(
        &self,
        root: &SqlExpr,
        access_chain: &[sqlparser::ast::AccessExpr],
    ) -> Result<TypedExpr> {
        // Check if this is $.column.path syntax
        if let SqlExpr::Value(value_with_span) = root {
            if matches!(value_with_span.value, Value::Placeholder(ref s) if s == "$") {
                // This is $.column.path syntax!
                let mut parts = Vec::new();
                for access in access_chain {
                    match access {
                        sqlparser::ast::AccessExpr::Dot(expr) => {
                            Self::extract_parts_from_access(expr, &mut parts)
                                .map_err(AnalysisError::UnsupportedExpression)?;
                        }
                        sqlparser::ast::AccessExpr::Subscript(subscript) => {
                            parts.push(Self::subscript_to_part(subscript)?);
                        }
                    }
                }

                if parts.is_empty() {
                    return Err(AnalysisError::UnsupportedExpression(
                        "$.syntax requires at least a column name: $.column or $.column.path"
                            .into(),
                    ));
                }

                let column_name = parts[0].clone();
                let json_path = if parts.len() > 1 {
                    let mut path = String::from("$");
                    for part in &parts[1..] {
                        if part.starts_with('[') {
                            path.push_str(part);
                        } else {
                            path.push('.');
                            path.push_str(part);
                        }
                    }
                    path
                } else {
                    "$".to_string()
                };

                return self.expand_dollar_dot_json_access_from_parts(&column_name, &json_path);
            }
        }

        Err(AnalysisError::UnsupportedExpression(format!("{:?}", root)))
    }

    /// Extract parts from a compound field access expression
    fn extract_parts_from_access(
        expr: &SqlExpr,
        parts: &mut Vec<String>,
    ) -> std::result::Result<(), String> {
        match expr {
            SqlExpr::Identifier(ident) => {
                parts.push(ident.value.clone());
                Ok(())
            }
            SqlExpr::CompoundFieldAccess { root, access_chain } => {
                Self::extract_parts_from_access(root, parts)?;
                for access in access_chain {
                    if let sqlparser::ast::AccessExpr::Dot(expr) = access {
                        Self::extract_parts_from_access(expr, parts)?;
                    } else {
                        return Err("$.syntax only supports dot notation".to_string());
                    }
                }
                Ok(())
            }
            SqlExpr::Function(func) => {
                if matches!(&func.args, FunctionArguments::None) {
                    let func_name = func.name.to_string();
                    parts.push(func_name);
                    return Ok(());
                }
                Err("$.syntax only supports simple paths, not function calls".to_string())
            }
            _ => Err(format!(
                "$.syntax only supports identifiers, got: {:?}",
                expr
            )),
        }
    }

    /// Convert a subscript to a path part string
    fn subscript_to_part(
        subscript: &sqlparser::ast::Subscript,
    ) -> std::result::Result<String, AnalysisError> {
        use sqlparser::ast::{Subscript, UnaryOperator};

        match subscript {
            Subscript::Index { index } => match index {
                SqlExpr::Value(value_with_span) => match &value_with_span.value {
                    Value::Number(n, _) => Ok(format!("[{}]", n)),
                    Value::SingleQuotedString(s) | Value::DoubleQuotedString(s) => Ok(s.clone()),
                    _ => Err(AnalysisError::UnsupportedExpression(
                        "Unsupported subscript value type".into(),
                    )),
                },
                SqlExpr::UnaryOp {
                    op: UnaryOperator::Minus,
                    expr,
                } => {
                    let SqlExpr::Value(value_with_span) = &**expr else {
                        return Err(AnalysisError::UnsupportedExpression(
                            "Unsupported negative subscript expression".into(),
                        ));
                    };
                    let Value::Number(n, _) = &value_with_span.value else {
                        return Err(AnalysisError::UnsupportedExpression(
                            "Unsupported negative subscript".into(),
                        ));
                    };
                    Ok(format!("[-{}]", n))
                }
                _ => Err(AnalysisError::UnsupportedExpression(
                    "$.syntax only supports literal subscripts".into(),
                )),
            },
            Subscript::Slice { .. } => Err(AnalysisError::UnsupportedExpression(
                "$.syntax does not support array slicing".into(),
            )),
        }
    }

    /// Parse JSONPath string into array path components
    pub(in crate::analyzer::semantic) fn parse_jsonpath_to_array_components(
        jsonpath: &str,
    ) -> Vec<String> {
        let path = jsonpath
            .strip_prefix("$.")
            .unwrap_or(jsonpath.strip_prefix("$").unwrap_or(jsonpath));

        let mut components = Vec::new();
        let mut current = String::new();
        let mut in_bracket = false;

        for ch in path.chars() {
            match ch {
                '.' if !in_bracket => {
                    if !current.is_empty() {
                        components.push(current.clone());
                        current.clear();
                    }
                }
                '[' => {
                    if !current.is_empty() {
                        components.push(current.clone());
                        current.clear();
                    }
                    in_bracket = true;
                }
                ']' => {
                    if !current.is_empty() {
                        components.push(current.clone());
                        current.clear();
                    }
                    in_bracket = false;
                }
                _ => {
                    current.push(ch);
                }
            }
        }

        if !current.is_empty() {
            components.push(current);
        }

        components
    }

    /// Expand $.column.path syntax into column::JSONB #> ARRAY['path', ...]
    pub(in crate::analyzer::semantic) fn expand_dollar_dot_json_access(
        &self,
        idents: &[Ident],
    ) -> Result<TypedExpr> {
        let first_ident = &idents[0].value;

        let (column_name, path_start_idx) = if first_ident == "$." {
            if idents.len() < 2 {
                return Err(AnalysisError::UnsupportedExpression(
                    "$.syntax requires at least a column name: $.column or $.column.path".into(),
                ));
            }
            (idents[1].value.clone(), 2)
        } else if let Some(col) = first_ident.strip_prefix("$.") {
            (col.to_string(), 1)
        } else {
            return Err(AnalysisError::UnsupportedExpression(
                "Invalid $.syntax".into(),
            ));
        };

        let json_path = if path_start_idx < idents.len() {
            let path_parts: Vec<&str> = idents[path_start_idx..]
                .iter()
                .map(|i| i.value.as_str())
                .collect();
            format!("$.{}", path_parts.join("."))
        } else {
            // Just column, no path: $.column -> return the column itself
            let column_sql_expr = SqlExpr::Identifier(sqlparser::ast::Ident::new(&column_name));
            return self.analyze_expr(&column_sql_expr);
        };

        let path_components = Self::parse_jsonpath_to_array_components(&json_path);

        // Build: CAST(column AS JSONB)
        let column_sql_expr = SqlExpr::Identifier(sqlparser::ast::Ident::new(&column_name));
        let column_expr = self.analyze_expr(&column_sql_expr)?;

        let cast_column = TypedExpr::new(
            Expr::Cast {
                expr: Box::new(column_expr),
                target_type: DataType::JsonB,
            },
            DataType::JsonB,
        );

        // Build: JSONB array literal
        let json_array_values: Vec<serde_json::Value> = path_components
            .iter()
            .map(|comp| serde_json::Value::String(comp.clone()))
            .collect();

        let path_array = TypedExpr::new(
            Expr::Literal(Literal::JsonB(serde_json::Value::Array(json_array_values))),
            DataType::JsonB,
        );

        // Build: column::JSONB #> ARRAY[...]
        Ok(TypedExpr::new(
            Expr::JsonExtractPath {
                object: Box::new(cast_column),
                path: Box::new(path_array),
            },
            DataType::Nullable(Box::new(DataType::JsonB)),
        ))
    }

    /// Helper for expand_dollar_dot_json_access that works with already-parsed parts
    pub(in crate::analyzer::semantic) fn expand_dollar_dot_json_access_from_parts(
        &self,
        column_name: &str,
        json_path: &str,
    ) -> Result<TypedExpr> {
        // Special case: if json_path is "$", just return the column itself
        if json_path == "$" {
            let column_sql_expr = SqlExpr::Identifier(sqlparser::ast::Ident::new(column_name));
            return self.analyze_expr(&column_sql_expr);
        }

        let path_components = Self::parse_jsonpath_to_array_components(json_path);

        // Build: CAST(column AS JSONB)
        let column_sql_expr = SqlExpr::Identifier(sqlparser::ast::Ident::new(column_name));
        let column_expr = self.analyze_expr(&column_sql_expr)?;

        let cast_column = TypedExpr::new(
            Expr::Cast {
                expr: Box::new(column_expr),
                target_type: DataType::JsonB,
            },
            DataType::JsonB,
        );

        // Build: JSONB array literal
        let json_array_values: Vec<serde_json::Value> = path_components
            .iter()
            .map(|comp| serde_json::Value::String(comp.clone()))
            .collect();

        let path_array = TypedExpr::new(
            Expr::Literal(Literal::JsonB(serde_json::Value::Array(json_array_values))),
            DataType::JsonB,
        );

        // Build: column::JSONB #> ARRAY[...]
        Ok(TypedExpr::new(
            Expr::JsonExtractPath {
                object: Box::new(cast_column),
                path: Box::new(path_array),
            },
            DataType::Nullable(Box::new(DataType::JsonB)),
        ))
    }
}
