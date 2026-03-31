//! Identifier analysis
//!
//! This module handles the analysis and resolution of identifiers including:
//! - Simple identifiers (column references)
//! - Compound identifiers (table.column references)

use crate::analyzer::{
    catalog::ColumnDef,
    error::AnalysisError,
    semantic::{types::TableRef, AnalyzerContext, Result},
    typed_expr::TypedExpr,
    types::DataType,
};
use sqlparser::ast::Ident;

impl<'a> AnalyzerContext<'a> {
    /// Analyze a simple identifier (column reference)
    pub(in crate::analyzer::semantic) fn analyze_identifier(
        &self,
        ident: &Ident,
    ) -> Result<TypedExpr> {
        let col_name = &ident.value;

        // Search for column in all current tables
        let mut found_columns: Vec<(TableRef, ColumnDef)> = Vec::new();

        for table_ref in &self.current_tables {
            // Check if this is a LATERAL function - it exposes a single virtual column
            if let Some(lateral_fn) = &table_ref.lateral_function {
                if col_name == &lateral_fn.column_name || col_name == table_ref.name() {
                    found_columns.push((
                        table_ref.clone(),
                        ColumnDef {
                            name: lateral_fn.column_name.clone(),
                            data_type: lateral_fn.return_type.clone(),
                            nullable: true,
                            generated: None,
                        },
                    ));
                }
                continue;
            }

            // Check if this is a subquery - use its schema directly
            let table = if let Some(subquery_ref) = &table_ref.subquery {
                subquery_ref.schema.clone()
            } else {
                self.get_table_def(&table_ref.table)?
                    .ok_or_else(|| AnalysisError::TableNotFound(table_ref.table.clone()))?
            };

            if let Some(col) = table.get_column(col_name) {
                found_columns.push((table_ref.clone(), col.clone()));
            } else if table.columns.is_empty() {
                // Table has dynamic schema (e.g., CYPHER function)
                found_columns.push((
                    table_ref.clone(),
                    ColumnDef {
                        name: col_name.clone(),
                        data_type: DataType::Text,
                        nullable: true,
                        generated: None,
                    },
                ));
            }
        }

        match found_columns.len() {
            0 => Err(AnalysisError::ColumnNotFound {
                table: if self.current_tables.is_empty() {
                    "unknown".to_string()
                } else {
                    self.current_tables
                        .iter()
                        .map(|t| t.table.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                column: col_name.clone(),
            }),
            1 => {
                let (table_ref, col) = &found_columns[0];
                Ok(TypedExpr::column(
                    table_ref.name().to_string(),
                    col.name.clone(),
                    col.data_type.clone(),
                ))
            }
            _ => Err(AnalysisError::AmbiguousColumn(format!(
                "Column '{}' is ambiguous. Did you mean {}?",
                col_name,
                found_columns
                    .iter()
                    .map(|(t, _)| format!("{}.{}", t.table, col_name))
                    .collect::<Vec<_>>()
                    .join(" or ")
            ))),
        }
    }

    /// Analyze a compound identifier (table.column reference)
    pub(in crate::analyzer::semantic) fn analyze_compound_identifier(
        &self,
        idents: &[Ident],
    ) -> Result<TypedExpr> {
        // Check for $.column.path JSON access syntax
        if idents.len() >= 2 {
            let first_ident = &idents[0].value;
            if first_ident == "$." || first_ident.starts_with("$.") {
                return self.expand_dollar_dot_json_access(idents);
            }
        }

        if idents.len() != 2 {
            return Err(AnalysisError::UnsupportedExpression(
                "Only table.column references supported".into(),
            ));
        }

        let table_or_alias = &idents[0].value;
        let col_name = &idents[1].value;

        // Search for table/alias in all current tables
        for table_ref in &self.current_tables {
            if table_ref.name() == table_or_alias {
                // Check if this is a LATERAL function reference
                if let Some(lateral_fn) = &table_ref.lateral_function {
                    // LATERAL functions expose a single column with the alias name
                    // Allow accessing it by any column name (for JSON extraction like res->>'key')
                    return Ok(TypedExpr::column(
                        table_ref.name().to_string(),
                        lateral_fn.column_name.clone(),
                        lateral_fn.return_type.clone(),
                    ));
                }

                let table = if let Some(subquery_ref) = &table_ref.subquery {
                    subquery_ref.schema.clone()
                } else {
                    self.get_table_def(&table_ref.table)?
                        .ok_or_else(|| AnalysisError::TableNotFound(table_ref.table.clone()))?
                };

                if let Some(col) = table.get_column(col_name) {
                    return Ok(TypedExpr::column(
                        table_ref.name().to_string(),
                        col.name.clone(),
                        col.data_type.clone(),
                    ));
                } else if table.columns.is_empty() {
                    // Table has dynamic schema
                    return Ok(TypedExpr::column(
                        table_ref.name().to_string(),
                        col_name.clone(),
                        DataType::Text,
                    ));
                }
            }
        }

        Err(AnalysisError::ColumnNotFound {
            table: table_or_alias.clone(),
            column: col_name.clone(),
        })
    }
}
