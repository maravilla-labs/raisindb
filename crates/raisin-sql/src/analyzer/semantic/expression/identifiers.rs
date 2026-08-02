//! Identifier analysis
//!
//! This module handles the analysis and resolution of identifiers including:
//! - Simple identifiers (column references)
//! - Compound identifiers (table.column references)

use crate::analyzer::{
    catalog::{ColumnDef, TableDef},
    error::AnalysisError,
    semantic::{types::TableRef, AnalyzerContext, Result},
    typed_expr::TypedExpr,
    types::DataType,
};
use sqlparser::ast::Ident;

/// Resolve an identifier that is not a declared column of `table`.
///
/// Node-backed tables (`nodes`, workspace tables, and the search/graph table
/// functions) carry a dynamic property bag in their `properties` JSONB column,
/// so a bare identifier that names no declared column names a *property*:
/// `SELECT user_id FROM nodes` means `properties->>'user_id'`.
///
/// This is not a convenience — it is the contract the rest of the engine already
/// implements. The scan layer materialises an unrecognised projection column out
/// of `node.properties` (`node_to_row/fields.rs::insert_property_fields`), scan
/// planning maps a bare column name straight to a property name
/// (`planner/scan_planning/build_scan.rs`), and row evaluation looks the
/// qualified name up in the row (`eval/core/mod.rs::eval_column`). Only the
/// analyzer rejected it, so the whole capability was unreachable — qualified
/// (`u.user_id`) *and* unqualified alike.
///
/// The type is `Unknown` because a property's type is not known until the row is
/// read; `Unknown` coerces to and from everything, which is what lets
/// `user_id = 5` and `user_id = 'abc'` both analyze.
fn dynamic_property_column(table: &TableDef, col_name: &str) -> Option<ColumnDef> {
    // Only tables that actually carry a property bag. A table without a
    // `properties` column (pg_catalog, a CTE, a subquery, information_schema)
    // keeps strict column resolution and still errors on a typo.
    if !matches!(
        table.get_column("properties").map(|c| &c.data_type),
        Some(DataType::JsonB)
    ) {
        return None;
    }

    // `__`-prefixed names are engine pseudo-columns (`__branch`, `__workspace`,
    // `__order`, …), not user properties. Leave them to whatever declares them.
    if col_name.starts_with("__") {
        return None;
    }

    Some(ColumnDef {
        name: col_name.to_string(),
        data_type: DataType::Unknown,
        nullable: true,
        generated: None,
    })
}

impl<'a> AnalyzerContext<'a> {
    /// Analyze a simple identifier (column reference)
    pub(in crate::analyzer::semantic) fn analyze_identifier(
        &self,
        ident: &Ident,
    ) -> Result<TypedExpr> {
        let col_name = &ident.value;

        // Search for column in all current tables
        let mut found_columns: Vec<(TableRef, ColumnDef)> = Vec::new();
        // Node-backed tables that could supply this name as a JSON property.
        // Only consulted when NO table declares the column, so a real column of a
        // joined CTE/subquery is never made ambiguous by a property fallback.
        let mut property_candidates: Vec<(TableRef, ColumnDef)> = Vec::new();

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
            } else if let Some(prop_col) = dynamic_property_column(&table, col_name) {
                property_candidates.push((table_ref.clone(), prop_col));
            }
        }

        if found_columns.is_empty() {
            found_columns = property_candidates;
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
                } else if let Some(prop_col) = dynamic_property_column(&table, col_name) {
                    // Not a declared column of a node-backed table: a JSON
                    // property, qualified by the table name or alias.
                    return Ok(TypedExpr::column(
                        table_ref.name().to_string(),
                        prop_col.name,
                        prop_col.data_type,
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
