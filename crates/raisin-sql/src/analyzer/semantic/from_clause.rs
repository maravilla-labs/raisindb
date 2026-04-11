//! FROM clause analysis
//!
//! This module handles the analysis of FROM clause elements including:
//! - Table references
//! - Table-valued functions (CYPHER, KNN, NEIGHBORS, FULLTEXT_SEARCH, GRAPH_TABLE)
//! - JOIN clauses
//! - Subqueries in FROM clause (derived tables)

use super::types::{
    JoinInfo, JoinType, LateralFunctionRef, SubqueryRef, TableFunctionRef, TableRef,
};
use super::{AnalyzerContext, Result};
use crate::analyzer::{
    catalog::{ColumnDef, TableDef},
    error::AnalysisError,
    functions::FunctionSignature,
    typed_expr::{Expr, Literal, TypedExpr},
    types::DataType,
};
use sqlparser::ast::{
    FunctionArg, FunctionArgExpr, TableFactor, TableFunctionArgs, TableWithJoins,
};

impl<'a> AnalyzerContext<'a> {
    /// Analyze FROM clause with support for multiple tables and joins
    pub(super) fn analyze_from_clause(
        &mut self,
        from_clause: &[TableWithJoins],
    ) -> Result<(Vec<TableRef>, Vec<JoinInfo>)> {
        let mut all_tables = Vec::new();
        let mut all_joins = Vec::new();

        for (idx, table_with_joins) in from_clause.iter().enumerate() {
            // For LATERAL functions (idx > 0), set current_tables so outer column references resolve
            let is_lateral_function = matches!(
                &table_with_joins.relation,
                TableFactor::Function { lateral: true, .. }
            );
            let saved_tables = if is_lateral_function && idx > 0 {
                let saved = self.current_tables.clone();
                self.current_tables = all_tables.clone();
                self.current_tables
                    .extend(all_joins.iter().map(|j: &JoinInfo| j.right_table.clone()));
                Some(saved)
            } else {
                None
            };

            // Extract the base table
            let base_table = self.analyze_table_factor(&table_with_joins.relation)?;

            // Restore current_tables if we modified them for LATERAL scope
            if let Some(saved) = saved_tables {
                self.current_tables = saved;
            }

            if idx == 0 {
                all_tables.push(base_table);
            } else {
                // Subsequent comma-separated tables are treated as CROSS JOINs
                all_joins.push(JoinInfo {
                    join_type: JoinType::Cross,
                    right_table: base_table,
                    condition: None,
                });
            }

            // Process explicit JOIN clauses
            for join in &table_with_joins.joins {
                use sqlparser::ast::JoinOperator;

                let right_table = self.analyze_table_factor(&join.relation)?;

                // Temporarily add both left and right tables to current_tables for ON clause analysis
                let saved_current_tables = self.current_tables.clone();
                self.current_tables = all_tables.clone();
                self.current_tables
                    .extend(all_joins.iter().map(|j| j.right_table.clone()));
                self.current_tables.push(right_table.clone());

                // Determine join type and condition
                let (join_type, condition) = match &join.join_operator {
                    JoinOperator::Join(constraint) => {
                        (JoinType::Inner, self.analyze_join_constraint(constraint)?)
                    }
                    JoinOperator::Inner(constraint) => {
                        (JoinType::Inner, self.analyze_join_constraint(constraint)?)
                    }
                    JoinOperator::Left(constraint) | JoinOperator::LeftOuter(constraint) => {
                        (JoinType::Left, self.analyze_join_constraint(constraint)?)
                    }
                    JoinOperator::Right(constraint) | JoinOperator::RightOuter(constraint) => {
                        (JoinType::Right, self.analyze_join_constraint(constraint)?)
                    }
                    JoinOperator::FullOuter(constraint) => {
                        (JoinType::Full, self.analyze_join_constraint(constraint)?)
                    }
                    JoinOperator::CrossJoin(_) => (JoinType::Cross, None),
                    _ => {
                        return Err(AnalysisError::UnsupportedStatement(format!(
                            "Unsupported join operator: {:?}",
                            join.join_operator
                        )))
                    }
                };

                // Restore current_tables
                self.current_tables = saved_current_tables;

                all_joins.push(JoinInfo {
                    join_type,
                    right_table,
                    condition,
                });
            }
        }

        Ok((all_tables, all_joins))
    }

    /// Analyze a table factor (table reference)
    pub(super) fn analyze_table_factor(&mut self, table_factor: &TableFactor) -> Result<TableRef> {
        match table_factor {
            TableFactor::Table {
                name, alias, args, ..
            } => {
                let table_name = name
                    .0
                    .iter()
                    .filter_map(|part| part.as_ident().map(|i| i.value.as_str()))
                    .collect::<Vec<_>>()
                    .join(".");

                let alias_str = alias.as_ref().map(|a| a.name.value.clone());

                // Check if this is a table-valued function call (has arguments)
                if args.is_some() {
                    return self.analyze_table_function(
                        &table_name,
                        alias_str,
                        args.as_ref().unwrap(),
                    );
                }

                // Regular table reference (no arguments)
                self.analyze_regular_table(&table_name, alias_str)
            }

            TableFactor::Derived {
                lateral,
                subquery,
                alias,
            } => self.analyze_derived_table(*lateral, subquery, alias),

            TableFactor::Function {
                lateral,
                name,
                args,
                alias,
            } => self.analyze_lateral_function(*lateral, name, args, alias),

            _ => Err(AnalysisError::UnsupportedStatement(format!(
                "Unsupported table reference type: {:?}",
                table_factor
            ))),
        }
    }

    /// Analyze a table-valued function
    fn analyze_table_function(
        &self,
        table_name: &str,
        alias_str: Option<String>,
        args: &TableFunctionArgs,
    ) -> Result<TableRef> {
        let table_upper = table_name.to_uppercase();

        // Check if it's a known table-valued function
        if !matches!(
            table_upper.as_str(),
            "CYPHER" | "KNN" | "NEIGHBORS" | "FULLTEXT_SEARCH" | "HYBRID_SEARCH" | "GRAPH_TABLE"
        ) {
            return Err(AnalysisError::UnsupportedStatement(format!(
                "Unsupported table-valued function: {}",
                table_name
            )));
        }

        let function_args = self.analyze_table_function_args(args)?;

        // For GRAPH_TABLE, build dynamic schema from COLUMNS clause
        let function_schema = if table_upper == "GRAPH_TABLE" {
            self.build_graph_table_schema(&function_args)?
        } else {
            self.get_table_def(table_name)?.ok_or_else(|| {
                AnalysisError::UnsupportedStatement(format!(
                    "Unsupported table-valued function: {}",
                    table_name
                ))
            })?
        };

        Ok(TableRef {
            table: table_name.to_string(),
            alias: alias_str,
            workspace: None,
            table_function: Some(TableFunctionRef {
                name: table_name.to_string(),
                args: function_args,
                schema: function_schema,
            }),
            subquery: None,
            lateral_function: None,
        })
    }

    /// Analyze a regular table reference
    fn analyze_regular_table(
        &self,
        table_name: &str,
        alias_str: Option<String>,
    ) -> Result<TableRef> {
        // Parse schema-qualified name (e.g., "pg_catalog.pg_type")
        let (schema_opt, simple_table_name) = if table_name.contains('.') {
            let parts: Vec<&str> = table_name.splitn(2, '.').collect();
            (Some(parts[0]), parts[1])
        } else {
            (None, table_name)
        };

        // Check if this is a pg_catalog system table
        if crate::analyzer::pg_catalog::is_pg_catalog_table(schema_opt, simple_table_name)
            && crate::analyzer::pg_catalog::get_pg_catalog_table(simple_table_name).is_some()
        {
            return Ok(TableRef {
                table: simple_table_name.to_string(),
                alias: alias_str,
                workspace: None,
                table_function: None,
                subquery: None,
                lateral_function: None,
            });
        }

        // First check if it's a CTE (Common Table Expression)
        if self.cte_catalog.contains_key(table_name) {
            return Ok(TableRef {
                table: table_name.to_string(),
                alias: alias_str,
                workspace: None,
                table_function: None,
                subquery: None,
                lateral_function: None,
            });
        }

        // Check if table exists in catalog as a regular table
        if self.catalog.get_table(table_name).is_some() {
            return Ok(TableRef {
                table: table_name.to_string(),
                alias: alias_str,
                workspace: None,
                table_function: None,
                subquery: None,
                lateral_function: None,
            });
        }

        // Check if it's a workspace table (dynamic workspace support)
        if self.catalog.get_workspace_table(table_name).is_some() {
            let workspace_name = self
                .catalog
                .resolve_workspace_name(table_name)
                .unwrap_or_else(|| table_name.to_string());

            return Ok(TableRef {
                table: table_name.to_string(),
                alias: alias_str,
                workspace: Some(workspace_name),
                table_function: None,
                subquery: None,
                lateral_function: None,
            });
        }

        // Table not found
        Err(AnalysisError::TableNotFound(table_name.to_string()))
    }

    /// Analyze a derived table (subquery in FROM clause)
    fn analyze_derived_table(
        &mut self,
        lateral: bool,
        subquery: &sqlparser::ast::Query,
        alias: &Option<sqlparser::ast::TableAlias>,
    ) -> Result<TableRef> {
        let subquery_analyzed = self.analyze_query(subquery)?;

        // Build schema from subquery projection
        let columns: Vec<ColumnDef> = subquery_analyzed
            .projection
            .iter()
            .map(|(expr, alias_opt)| {
                let col_name = alias_opt.clone().unwrap_or_else(|| match &expr.expr {
                    crate::analyzer::Expr::Column { column, .. } => column.clone(),
                    _ => "?column?".to_string(),
                });
                ColumnDef {
                    name: col_name,
                    data_type: expr.data_type.clone(),
                    nullable: true,
                    generated: None,
                }
            })
            .collect();

        // Get alias name (required for derived tables)
        let alias_name = alias
            .as_ref()
            .map(|a| a.name.value.clone())
            .ok_or_else(|| {
                AnalysisError::UnsupportedStatement("Derived tables must have an alias".to_string())
            })?;

        let schema = TableDef {
            name: alias_name.clone(),
            columns,
            primary_key: Vec::new(),
            indexes: Vec::new(),
        };

        let subquery_ref = SubqueryRef {
            query: Box::new(subquery_analyzed),
            schema: schema.clone(),
            is_lateral: lateral,
        };

        Ok(TableRef {
            table: alias_name.clone(),
            alias: Some(alias_name),
            workspace: None,
            table_function: None,
            subquery: Some(subquery_ref),
            lateral_function: None,
        })
    }

    /// Analyze a LATERAL function in FROM clause
    ///
    /// Handles `LATERAL func(args) AS alias` syntax where a scalar function
    /// is applied per-row to produce a new column.
    fn analyze_lateral_function(
        &self,
        lateral: bool,
        name: &sqlparser::ast::ObjectName,
        args: &[FunctionArg],
        alias: &Option<sqlparser::ast::TableAlias>,
    ) -> Result<TableRef> {
        if !lateral {
            return Err(AnalysisError::UnsupportedStatement(
                "Non-LATERAL function in FROM clause is not supported. Use LATERAL keyword."
                    .to_string(),
            ));
        }

        // Extract function name
        let func_name = name
            .0
            .iter()
            .filter_map(|part| part.as_ident().map(|i| i.value.as_str()))
            .collect::<Vec<_>>()
            .join(".")
            .to_uppercase();

        // Analyze function arguments as expressions
        let analyzed_args: Result<Vec<TypedExpr>> = args
            .iter()
            .map(|arg| match arg {
                FunctionArg::Unnamed(FunctionArgExpr::Expr(expr)) => self.analyze_expr(expr),
                FunctionArg::Named {
                    arg: FunctionArgExpr::Expr(expr),
                    ..
                } => self.analyze_expr(expr),
                _ => Err(AnalysisError::UnsupportedExpression(
                    "Unsupported argument type in LATERAL function".into(),
                )),
            })
            .collect();
        let analyzed_args = analyzed_args?;
        let arg_types: Vec<DataType> = analyzed_args.iter().map(|a| a.data_type.clone()).collect();

        // Resolve function signature from registry
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
            })?
            .clone();

        let return_type = signature.return_type.clone();

        // Build the function call expression
        let function_expr = TypedExpr::new(
            Expr::Function {
                name: func_name.clone(),
                args: analyzed_args,
                signature: FunctionSignature {
                    name: signature.name.clone(),
                    params: signature.params.clone(),
                    return_type: signature.return_type.clone(),
                    is_deterministic: signature.is_deterministic,
                    category: signature.category.clone(),
                },
                filter: None,
            },
            return_type.clone(),
        );

        // Extract alias (required for LATERAL functions)
        let alias_name = alias
            .as_ref()
            .map(|a| a.name.value.clone())
            .ok_or_else(|| {
                AnalysisError::UnsupportedStatement(
                    "LATERAL function requires an alias (e.g., LATERAL func(x) AS alias)"
                        .to_string(),
                )
            })?;

        Ok(TableRef {
            table: alias_name.clone(),
            alias: Some(alias_name.clone()),
            workspace: None,
            table_function: None,
            subquery: None,
            lateral_function: Some(LateralFunctionRef {
                function_expr,
                column_name: alias_name,
                return_type,
            }),
        })
    }

    /// Analyze table function arguments
    pub(super) fn analyze_table_function_args(
        &self,
        args: &TableFunctionArgs,
    ) -> Result<Vec<TypedExpr>> {
        if args.settings.is_some() {
            return Err(AnalysisError::UnsupportedExpression(
                "SETTINGS clause not supported for table-valued functions".into(),
            ));
        }

        let mut result = Vec::with_capacity(args.args.len());
        for arg in &args.args {
            match arg {
                FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))
                | FunctionArg::Named {
                    arg: FunctionArgExpr::Expr(expr),
                    ..
                } => {
                    result.push(self.analyze_expr(expr)?);
                }
                FunctionArg::Unnamed(FunctionArgExpr::Wildcard)
                | FunctionArg::Unnamed(FunctionArgExpr::QualifiedWildcard(_))
                | FunctionArg::Named {
                    arg: FunctionArgExpr::Wildcard,
                    ..
                }
                | FunctionArg::Named {
                    arg: FunctionArgExpr::QualifiedWildcard(_),
                    ..
                } => {
                    return Err(AnalysisError::UnsupportedExpression(
                        "Wildcard arguments not supported for table-valued functions".into(),
                    ))
                }
                FunctionArg::ExprNamed { .. } => {
                    return Err(AnalysisError::UnsupportedExpression(
                        "Named expression arguments not supported for table-valued functions"
                            .into(),
                    ))
                }
            }
        }
        Ok(result)
    }

    /// Build schema for GRAPH_TABLE from its COLUMNS clause
    pub(super) fn build_graph_table_schema(&self, args: &[TypedExpr]) -> Result<TableDef> {
        // GRAPH_TABLE argument should be a string literal containing the full query
        let query_str = if let Some(arg) = args.first() {
            match &arg.expr {
                Expr::Literal(Literal::Text(s)) => s.clone(),
                _ => {
                    // Fall back to static schema if we can't parse
                    return self.get_table_def("GRAPH_TABLE")?.ok_or_else(|| {
                        AnalysisError::UnsupportedStatement("Invalid GRAPH_TABLE argument".into())
                    });
                }
            }
        } else {
            return Err(AnalysisError::UnsupportedStatement(
                "GRAPH_TABLE requires an argument".into(),
            ));
        };

        // Parse the GRAPH_TABLE query to extract COLUMNS clause
        match crate::ast::pgq_parser::parse_graph_table(&query_str) {
            Ok(graph_table) => {
                let mut columns = Vec::new();

                for col_expr in &graph_table.columns_clause.columns {
                    let col_name = Self::get_pgq_column_name(col_expr);
                    let col_type = Self::infer_pgq_column_type(&col_expr.expr);

                    columns.push(ColumnDef {
                        name: col_name,
                        data_type: col_type,
                        nullable: true,
                        generated: None,
                    });
                }

                Ok(TableDef {
                    name: "GRAPH_TABLE".into(),
                    columns,
                    primary_key: vec![],
                    indexes: vec![],
                })
            }
            Err(_) => {
                // Fall back to static schema on parse error
                self.get_table_def("GRAPH_TABLE")?.ok_or_else(|| {
                    AnalysisError::UnsupportedStatement("Invalid GRAPH_TABLE syntax".into())
                })
            }
        }
    }

    /// Get column name from PGQ ColumnExpr
    fn get_pgq_column_name(col: &crate::ast::ColumnExpr) -> String {
        if let Some(alias) = &col.alias {
            return alias.clone();
        }

        match &col.expr {
            crate::ast::Expr::PropertyAccess {
                variable,
                properties,
                ..
            } => {
                if properties.is_empty() {
                    variable.clone()
                } else {
                    format!("{}_{}", variable, properties.join("_"))
                }
            }
            crate::ast::Expr::FunctionCall { name, .. } => name.to_lowercase(),
            crate::ast::Expr::Wildcard { qualifier, .. } => {
                qualifier.clone().unwrap_or_else(|| "*".into())
            }
            _ => "column".into(),
        }
    }

    /// Infer column type from PGQ expression
    fn infer_pgq_column_type(expr: &crate::ast::Expr) -> DataType {
        match expr {
            crate::ast::Expr::PropertyAccess { properties, .. } if properties.is_empty() => {
                DataType::JsonB
            }
            crate::ast::Expr::PropertyAccess { properties, .. } => {
                let prop = properties.first().map(|s| s.as_str()).unwrap_or("");
                match prop {
                    "id" | "workspace" | "node_type" | "path" | "name" => DataType::Text,
                    "properties" => DataType::JsonB,
                    "created_at" | "updated_at" => DataType::Text,
                    "weight" => DataType::Double,
                    "version" => DataType::BigInt,
                    _ => DataType::JsonB,
                }
            }
            crate::ast::Expr::FunctionCall { name, .. } => {
                let name_lower = name.to_lowercase();
                match name_lower.as_str() {
                    "count" => DataType::BigInt,
                    "sum" | "avg" => DataType::Double,
                    "min" | "max" => DataType::Unknown,
                    "collect" | "array_agg" => DataType::Array(Box::new(DataType::Unknown)),
                    _ => DataType::Unknown,
                }
            }
            crate::ast::Expr::Wildcard { .. } => DataType::Unknown,
            crate::ast::Expr::Literal(lit) => match lit {
                crate::ast::Literal::Integer(_) => DataType::BigInt,
                crate::ast::Literal::Float(_) => DataType::Double,
                crate::ast::Literal::String(_) => DataType::Text,
                crate::ast::Literal::Boolean(_) => DataType::Boolean,
                crate::ast::Literal::Null => DataType::Unknown,
            },
            _ => DataType::Unknown,
        }
    }

    /// Analyze join constraint (ON clause or USING clause)
    pub(super) fn analyze_join_constraint(
        &mut self,
        constraint: &sqlparser::ast::JoinConstraint,
    ) -> Result<Option<TypedExpr>> {
        use sqlparser::ast::JoinConstraint;

        match constraint {
            JoinConstraint::On(expr) => {
                let typed_expr = self.analyze_expr(expr)?;
                if !matches!(typed_expr.data_type.base_type(), DataType::Boolean) {
                    return Err(AnalysisError::TypeMismatch {
                        expected: "BOOLEAN".into(),
                        actual: typed_expr.data_type.to_string(),
                    });
                }
                Ok(Some(typed_expr))
            }
            JoinConstraint::Using(_) => Err(AnalysisError::UnsupportedStatement(
                "USING clause not yet supported".into(),
            )),
            JoinConstraint::Natural => Err(AnalysisError::UnsupportedStatement(
                "NATURAL JOIN not yet supported".into(),
            )),
            JoinConstraint::None => Ok(None),
        }
    }
}
