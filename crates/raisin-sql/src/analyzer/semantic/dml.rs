//! DML (Data Manipulation Language) Analysis
//!
//! This module provides semantic analysis for INSERT, UPDATE, and DELETE statements.
//! It validates table references, column names, type compatibility, and constructs
//! typed DML operations for execution.

use sqlparser::ast::{
    Assignment, AssignmentTarget, Delete as SqlDelete, Expr as SqlExpr, FromTable,
    Insert as SqlInsert, ObjectName, TableFactor, TableObject, TableWithJoins,
};

use super::{
    super::{
        catalog::{is_schema_table, SchemaTableKind, TableDef},
        error::{AnalysisError, Result},
    },
    predicates::extract_branch_predicate,
    types::{AnalyzedDelete, AnalyzedInsert, AnalyzedStatement, AnalyzedUpdate, DmlTableTarget},
    AnalyzerContext,
};

impl<'a> AnalyzerContext<'a> {
    /// Analyze an INSERT statement
    ///
    /// Validates:
    /// - Table exists and supports DML operations
    /// - Column names are valid for the target table
    /// - Value types match column types
    /// - Number of values matches number of columns
    ///
    /// For regular INSERT, pass `is_upsert = false`. This uses `add_node()` which
    /// fails if a node already exists at the path.
    ///
    /// For UPSERT statements, pass `is_upsert = true`. This uses `put_node()` which
    /// creates a new node or updates an existing one at the path.
    pub(super) fn analyze_insert(
        &mut self,
        insert: &SqlInsert,
        is_upsert: bool,
    ) -> Result<AnalyzedStatement> {
        // Extract table name from INSERT statement
        let table_name = extract_table_name_from_table_object(&insert.table)?;

        // Determine target and get schema
        let (target, schema) = self.resolve_dml_target(&table_name)?;

        // Extract column names if specified, otherwise use all columns in order
        let columns = if insert.columns.is_empty() {
            // No columns specified - use all columns in schema order
            schema
                .column_names()
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            // Validate specified columns exist in schema
            let mut cols = Vec::new();
            for col_ident in &insert.columns {
                let col_name = col_ident.value.clone();
                if schema.get_column(&col_name).is_none() {
                    return Err(AnalysisError::ColumnNotFound {
                        table: table_name.clone(),
                        column: col_name,
                    });
                }
                cols.push(col_name);
            }
            cols
        };

        // Analyze VALUES clause - each row should have values matching column count
        let mut typed_values = Vec::new();

        if let Some(source) = &insert.source {
            if let sqlparser::ast::SetExpr::Values(values) = &*source.body {
                for row in &values.rows {
                    if row.len() != columns.len() {
                        return Err(AnalysisError::InvalidArgumentCount {
                            function: format!("INSERT INTO {}", table_name),
                            expected: columns.len(),
                            actual: row.len(),
                        });
                    }

                    let mut typed_row = Vec::new();
                    for (idx, value_expr) in row.iter().enumerate() {
                        // Temporarily add the table to context for expression analysis
                        self.current_tables.push(super::TableRef {
                            table: table_name.clone(),
                            alias: None,
                            workspace: None,
                            table_function: None,
                            subquery: None,
                            lateral_function: None,
                        });

                        // Type-check the value expression
                        let typed_expr = self.analyze_expr(value_expr)?;

                        // Remove temporary table
                        self.current_tables.pop();

                        // Get expected column type
                        let col_name = &columns[idx];
                        let col_def = schema.get_column(col_name).ok_or_else(|| {
                            AnalysisError::InternalError(format!(
                                "Column {} disappeared during analysis",
                                col_name
                            ))
                        })?;

                        // Verify type compatibility (allow coercion)
                        if !typed_expr.data_type.can_coerce_to(&col_def.data_type) {
                            return Err(AnalysisError::TypeMismatch {
                                expected: col_def.data_type.to_string(),
                                actual: typed_expr.data_type.to_string(),
                            });
                        }

                        typed_row.push(typed_expr);
                    }
                    typed_values.push(typed_row);
                }
            } else {
                return Err(AnalysisError::UnsupportedStatement(
                    "INSERT with SELECT is not yet supported".to_string(),
                ));
            }
        } else {
            return Err(AnalysisError::UnsupportedStatement(
                "INSERT without VALUES clause is not supported".to_string(),
            ));
        }

        Ok(AnalyzedStatement::Insert(AnalyzedInsert {
            target,
            schema,
            columns,
            values: typed_values,
            is_upsert,
        }))
    }

    /// Analyze an UPDATE statement
    ///
    /// Validates:
    /// - Table exists and supports DML operations
    /// - Assignment column names are valid
    /// - Assignment value types match column types
    /// - WHERE clause is properly typed
    pub(super) fn analyze_update(
        &mut self,
        table: &TableWithJoins,
        assignments: &[Assignment],
        selection: Option<&SqlExpr>,
    ) -> Result<AnalyzedStatement> {
        // Extract table name from UPDATE statement
        let table_name = extract_table_name_from_table_with_joins(table)?;

        // Determine target and get schema
        let (target, schema) = self.resolve_dml_target(&table_name)?;

        // Add table to context for expression analysis
        self.current_tables.push(super::TableRef {
            table: table_name.clone(),
            alias: None,
            workspace: None,
            table_function: None,
            subquery: None,
            lateral_function: None,
        });

        // Analyze SET clause assignments
        let mut typed_assignments = Vec::new();
        for assignment in assignments {
            // Extract column name from assignment target
            let col_name = match &assignment.target {
                AssignmentTarget::ColumnName(name) => extract_table_name(name)?,
                AssignmentTarget::Tuple(_) => {
                    return Err(AnalysisError::UnsupportedExpression(
                        "Tuple assignments in SET clause not yet supported".to_string(),
                    ));
                }
            };

            // Verify column exists
            let col_def =
                schema
                    .get_column(&col_name)
                    .ok_or_else(|| AnalysisError::ColumnNotFound {
                        table: table_name.clone(),
                        column: col_name.clone(),
                    })?;

            // Type-check the value expression
            let typed_value = self.analyze_expr(&assignment.value)?;

            // Verify type compatibility (allow coercion)
            if !typed_value.data_type.can_coerce_to(&col_def.data_type) {
                return Err(AnalysisError::TypeMismatch {
                    expected: col_def.data_type.to_string(),
                    actual: typed_value.data_type.to_string(),
                });
            }

            typed_assignments.push((col_name, typed_value));
        }

        // Analyze WHERE clause if present
        let (filter, branch_override) = if let Some(where_expr) = selection {
            let typed_filter = self.analyze_expr(where_expr)?;
            // Extract __branch predicate from the filter
            let (branch, remaining_filter) = extract_branch_predicate(&typed_filter);
            (remaining_filter, branch)
        } else {
            (None, None)
        };

        // Remove table from context
        self.current_tables.pop();

        Ok(AnalyzedStatement::Update(AnalyzedUpdate {
            target,
            schema,
            assignments: typed_assignments,
            filter,
            branch_override,
        }))
    }

    /// Analyze a DELETE statement
    ///
    /// Validates:
    /// - Table exists and supports DML operations
    /// - WHERE clause is properly typed
    pub(super) fn analyze_delete(&mut self, delete: &SqlDelete) -> Result<AnalyzedStatement> {
        // Extract table name from DELETE statement's FROM clause
        let table_name = extract_table_name_from_delete(delete)?;

        // Determine target and get schema
        let (target, schema) = self.resolve_dml_target(&table_name)?;

        // Add table to context for WHERE clause analysis
        self.current_tables.push(super::TableRef {
            table: table_name.clone(),
            alias: None,
            workspace: None,
            table_function: None,
            subquery: None,
            lateral_function: None,
        });

        // Analyze WHERE clause if present
        let (filter, branch_override) = if let Some(where_expr) = &delete.selection {
            let typed_filter = self.analyze_expr(where_expr)?;
            // Extract __branch predicate from the filter
            let (branch, remaining_filter) = extract_branch_predicate(&typed_filter);
            (remaining_filter, branch)
        } else {
            (None, None)
        };

        // Remove table from context
        self.current_tables.pop();

        Ok(AnalyzedStatement::Delete(AnalyzedDelete {
            target,
            schema,
            filter,
            branch_override,
        }))
    }

    /// Resolve a table name to a DML target and schema
    ///
    /// Schema tables (NodeTypes, Archetypes, ElementTypes) must use DDL syntax.
    /// DML operations are not yet supported on regular workspace tables.
    fn resolve_dml_target(&self, table_name: &str) -> Result<(DmlTableTarget, TableDef)> {
        // Check if it's a schema table - these now require DDL syntax
        if is_schema_table(table_name) {
            let kind = SchemaTableKind::from_table_name(table_name).ok_or_else(|| {
                AnalysisError::InternalError(format!(
                    "is_schema_table returned true but from_table_name returned None for {}",
                    table_name
                ))
            })?;

            let ddl_syntax = match kind {
                SchemaTableKind::NodeTypes => "CREATE/ALTER/DROP NODETYPE",
                SchemaTableKind::Archetypes => "CREATE/ALTER/DROP ARCHETYPE",
                SchemaTableKind::ElementTypes => "CREATE/ALTER/DROP ELEMENTTYPE",
            };

            return Err(AnalysisError::UnsupportedStatement(format!(
                "Direct DML operations on '{}' are not allowed. Use DDL syntax instead: {}",
                kind.table_name(),
                ddl_syntax
            )));
        }

        // Check if it's a workspace table
        if let Some(schema) = self.catalog.get_workspace_table(table_name) {
            return Ok((DmlTableTarget::Workspace(table_name.to_string()), schema));
        }

        // Check if it's a regular table (not workspace)
        if self.catalog.get_table(table_name).is_some() {
            return Err(AnalysisError::UnsupportedStatement(format!(
                "DML operations are not yet supported on table '{}'",
                table_name
            )));
        }

        // Table doesn't exist
        Err(AnalysisError::TableNotFound(table_name.to_string()))
    }
}

/// Extract table name from TableObject (used in INSERT statements)
fn extract_table_name_from_table_object(table_obj: &TableObject) -> Result<String> {
    match table_obj {
        TableObject::TableName(name) => extract_table_name(name),
        TableObject::TableFunction(_) => Err(AnalysisError::UnsupportedStatement(
            "Table functions in INSERT are not supported".to_string(),
        )),
    }
}

/// Extract table name from a sqlparser ObjectName
///
/// ObjectName can be multi-part for qualified names (schema.table).
/// We only support single-part names currently.
fn extract_table_name(name: &ObjectName) -> Result<String> {
    let parts: Vec<String> = name
        .0
        .iter()
        .filter_map(|part| part.as_ident().map(|i| i.value.clone()))
        .collect();

    if parts.is_empty() {
        return Err(AnalysisError::UnsupportedStatement(
            "Empty table name".to_string(),
        ));
    }

    if parts.len() > 1 {
        return Err(AnalysisError::UnsupportedStatement(
            "Qualified table names (schema.table) are not yet supported".to_string(),
        ));
    }

    Ok(parts[0].clone())
}

/// Extract table name from TableWithJoins
///
/// TableWithJoins can contain a single table plus optional JOINs.
/// For DML operations, we only support single tables (no JOINs).
fn extract_table_name_from_table_with_joins(table_with_joins: &TableWithJoins) -> Result<String> {
    // Check for JOINs - not supported in DML
    if !table_with_joins.joins.is_empty() {
        return Err(AnalysisError::UnsupportedStatement(
            "JOINs in UPDATE/DELETE are not supported".to_string(),
        ));
    }

    // Extract table name from the relation (TableFactor)
    match &table_with_joins.relation {
        TableFactor::Table { name, .. } => extract_table_name(name),
        TableFactor::Derived { .. } => Err(AnalysisError::UnsupportedStatement(
            "Subqueries in UPDATE/DELETE are not supported".to_string(),
        )),
        TableFactor::TableFunction { .. } => Err(AnalysisError::UnsupportedStatement(
            "Table functions in UPDATE/DELETE are not supported".to_string(),
        )),
        _ => Err(AnalysisError::UnsupportedStatement(
            "Unsupported table reference in UPDATE/DELETE".to_string(),
        )),
    }
}

/// Extract table name from DELETE statement
///
/// Handles both standard `DELETE FROM table` syntax (in `from` field)
/// and MySQL multi-table DELETE syntax (in `tables` field).
fn extract_table_name_from_delete(delete: &SqlDelete) -> Result<String> {
    // First, try the standard FROM clause (used by PostgreSQL, SQLite, etc.)
    let from_tables = match &delete.from {
        FromTable::WithFromKeyword(tables) => tables,
        FromTable::WithoutKeyword(tables) => tables,
    };

    if !from_tables.is_empty() {
        // We only support single-table DELETE
        if from_tables.len() > 1 {
            return Err(AnalysisError::UnsupportedStatement(
                "Multi-table DELETE is not yet supported".to_string(),
            ));
        }
        return extract_table_name_from_table_with_joins(&from_tables[0]);
    }

    // Fall back to MySQL-style `tables` field
    if !delete.tables.is_empty() {
        if delete.tables.len() > 1 {
            return Err(AnalysisError::UnsupportedStatement(
                "Multi-table DELETE is not yet supported".to_string(),
            ));
        }
        return extract_table_name(&delete.tables[0]);
    }

    // Neither FROM nor tables specified
    Err(AnalysisError::UnsupportedStatement(
        "DELETE without FROM clause is not supported".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_table_name() {
        use sqlparser::ast::{Ident, ObjectName, ObjectNamePart};

        // Single part name
        let name = ObjectName(vec![ObjectNamePart::Identifier(Ident::new("NodeTypes"))]);
        assert_eq!(extract_table_name(&name).unwrap(), "NodeTypes");

        // Empty name should error
        let name = ObjectName(vec![]);
        assert!(extract_table_name(&name).is_err());

        // Multi-part name should error (for now)
        let name = ObjectName(vec![
            ObjectNamePart::Identifier(Ident::new("schema")),
            ObjectNamePart::Identifier(Ident::new("table")),
        ]);
        assert!(extract_table_name(&name).is_err());
    }

    #[test]
    fn test_schema_table_kind() {
        // Test case-insensitive parsing
        assert_eq!(
            SchemaTableKind::from_table_name("NodeTypes"),
            Some(SchemaTableKind::NodeTypes)
        );
        assert_eq!(
            SchemaTableKind::from_table_name("nodetypes"),
            Some(SchemaTableKind::NodeTypes)
        );
        assert_eq!(
            SchemaTableKind::from_table_name("NODETYPES"),
            Some(SchemaTableKind::NodeTypes)
        );

        // Test canonical names
        assert_eq!(SchemaTableKind::NodeTypes.table_name(), "NodeTypes");
        assert_eq!(SchemaTableKind::Archetypes.table_name(), "Archetypes");
        assert_eq!(SchemaTableKind::ElementTypes.table_name(), "ElementTypes");
    }
}
