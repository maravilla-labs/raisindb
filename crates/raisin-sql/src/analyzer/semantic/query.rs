//! Query and SELECT statement analysis
//!
//! This module handles the analysis of SQL queries including:
//! - Statement analysis (SELECT, INSERT, UPDATE, DELETE, EXPLAIN, SHOW)
//! - SELECT clause analysis
//! - CTE (Common Table Expression) analysis
//! - Projection analysis

use super::predicates::{
    extract_branch_predicate, extract_locale_predicate, extract_revision_predicate,
};
use super::types::{
    AnalyzedQuery, AnalyzedShow, AnalyzedStatement, CteDefinition, ExplainFormat, ExplainStatement,
};
use super::{AnalyzerContext, Result};
use crate::analyzer::{
    catalog::{ColumnDef, TableDef},
    error::AnalysisError,
    typed_expr::{Expr, TypedExpr},
    types::DataType,
};
use sqlparser::ast::{
    LimitClause, OrderByExpr, OrderByKind, Query, Select, SelectItem,
    SelectItemQualifiedWildcardKind, SetExpr, Statement,
};

impl<'a> AnalyzerContext<'a> {
    /// Analyze a SQL statement
    pub fn analyze_statement(&mut self, stmt: &Statement) -> Result<AnalyzedStatement> {
        match stmt {
            Statement::Query(query) => Ok(AnalyzedStatement::Query(self.analyze_query(query)?)),
            Statement::Explain {
                statement,
                analyze,
                verbose,
                format,
                ..
            } => self.analyze_explain(statement, *analyze, *verbose, format.as_ref()),
            Statement::Insert(insert) => self.analyze_insert(insert, self.is_upsert),
            Statement::Update {
                table,
                assignments,
                selection,
                ..
            } => self.analyze_update(table, assignments, selection.as_ref()),
            Statement::Delete(delete) => self.analyze_delete(delete),
            Statement::ShowVariable { variable } => self.analyze_show_variable(variable),
            _ => Err(AnalysisError::UnsupportedStatement(format!("{:?}", stmt))),
        }
    }

    /// Analyze SHOW VARIABLE statement
    pub(super) fn analyze_show_variable(
        &self,
        variable: &[sqlparser::ast::Ident],
    ) -> Result<AnalyzedStatement> {
        let variable_name = variable
            .iter()
            .map(|ident| ident.value.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");

        Ok(AnalyzedStatement::Show(AnalyzedShow {
            variable: variable_name,
        }))
    }

    /// Analyze EXPLAIN statement
    pub(super) fn analyze_explain(
        &mut self,
        statement: &Statement,
        analyze: bool,
        verbose: bool,
        _format: Option<&sqlparser::ast::AnalyzeFormatKind>,
    ) -> Result<AnalyzedStatement> {
        let explain_format = ExplainFormat::Text;

        match statement {
            Statement::Query(query) => {
                let analyzed_query = self.analyze_query(query)?;
                Ok(AnalyzedStatement::Explain(ExplainStatement {
                    query: Box::new(analyzed_query),
                    analyze,
                    format: explain_format,
                    verbose,
                }))
            }
            _ => Err(AnalysisError::UnsupportedStatement(
                "EXPLAIN only supports SELECT queries".into(),
            )),
        }
    }

    /// Analyze a query (WITH ... SELECT ...)
    pub(super) fn analyze_query(&mut self, query: &Query) -> Result<AnalyzedQuery> {
        // Analyze CTEs first (WITH clause)
        let ctes = if let Some(with) = &query.with {
            self.analyze_ctes(with)?
        } else {
            Vec::new()
        };

        // Extract order_by, limit, and offset from the query
        let order_by_exprs = if let Some(order_by) = &query.order_by {
            match &order_by.kind {
                OrderByKind::Expressions(exprs) => exprs.as_slice(),
                _ => &[],
            }
        } else {
            &[]
        };

        let (limit_expr, offset_expr) = match &query.limit_clause {
            Some(LimitClause::LimitOffset {
                limit,
                offset,
                limit_by: _,
            }) => (limit.as_ref(), offset.as_ref().map(|o| &o.value)),
            _ => (None, None),
        };

        let set_expr = &query.body;
        let mut analyzed = match set_expr.as_ref() {
            SetExpr::Select(select) => {
                self.analyze_select(select, order_by_exprs, limit_expr, offset_expr)?
            }
            _ => {
                return Err(AnalysisError::UnsupportedStatement(
                    "Only SELECT queries are supported".into(),
                ))
            }
        };

        // Add CTEs to the analyzed query
        analyzed.ctes = ctes;
        Ok(analyzed)
    }

    /// Analyze a SELECT statement
    pub(super) fn analyze_select(
        &mut self,
        select: &Select,
        order_by: &[OrderByExpr],
        limit: Option<&sqlparser::ast::Expr>,
        offset: Option<&sqlparser::ast::Expr>,
    ) -> Result<AnalyzedQuery> {
        // Validate no unsupported features
        if select.having.is_some() {
            return Err(AnalysisError::UnsupportedStatement(
                "HAVING not yet supported".into(),
            ));
        }

        // Analyze FROM clause
        let (tables, joins) = if select.from.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            let (tables, joins) = self.analyze_from_clause(&select.from)?;
            self.current_tables = tables.clone();
            self.current_tables
                .extend(joins.iter().map(|j| j.right_table.clone()));
            (tables, joins)
        };

        // Analyze WHERE clause
        let selection = if let Some(where_expr) = &select.selection {
            let typed_expr = self.analyze_expr(where_expr)?;
            if !matches!(typed_expr.data_type.base_type(), DataType::Boolean) {
                return Err(AnalysisError::TypeMismatch {
                    expected: "BOOLEAN".into(),
                    actual: typed_expr.data_type.to_string(),
                });
            }
            Some(typed_expr)
        } else {
            None
        };

        // Analyze SELECT list
        let projection = self.analyze_projection(&select.projection)?;

        // Analyze GROUP BY expressions
        let group_by = self.analyze_group_by(&select.group_by)?;

        // Extract aggregate functions from projection
        let mut aggregates = Vec::new();
        let mut has_aggregates = false;
        for (expr, alias) in &projection {
            if let Some(agg_exprs) = self.extract_aggregates(expr, alias.as_deref())? {
                aggregates.extend(agg_exprs);
                has_aggregates = true;
            }
        }

        // Validate GROUP BY usage
        if has_aggregates || !group_by.is_empty() {
            self.validate_grouping(&projection, &group_by, &aggregates)?;
        }

        // Build alias map for ORDER BY resolution
        let alias_map: std::collections::HashMap<String, TypedExpr> = projection
            .iter()
            .filter_map(|(expr, alias)| alias.as_ref().map(|a| (a.clone(), expr.clone())))
            .collect();

        // Analyze ORDER BY (with alias resolution)
        let order_by_analyzed = self.analyze_order_by(order_by, &alias_map)?;

        // Analyze LIMIT and OFFSET
        let limit_val = if let Some(limit_expr) = limit {
            Some(self.analyze_limit(limit_expr)?)
        } else {
            None
        };

        let offset_val = if let Some(offset_expr) = offset {
            Some(self.analyze_offset(offset_expr)?)
        } else {
            None
        };

        // Extract __revision predicate from selection
        let (max_revision, remaining_selection) = if let Some(sel) = selection {
            extract_revision_predicate(&sel)
        } else {
            (None, None)
        };

        // Extract __branch predicate from remaining selection
        let (branch_override, remaining_selection2) = if let Some(sel) = remaining_selection {
            extract_branch_predicate(&sel)
        } else {
            (None, None)
        };

        // Extract locale predicate from remaining selection
        let (locales, final_selection) = if let Some(sel) = remaining_selection2 {
            extract_locale_predicate(&sel)
        } else {
            (vec![], None)
        };

        // Analyze DISTINCT clause
        let distinct = self.analyze_distinct(&select.distinct, &projection, order_by)?;

        Ok(AnalyzedQuery {
            ctes: Vec::new(),
            projection,
            from: tables,
            joins,
            selection: final_selection,
            group_by,
            aggregates,
            order_by: order_by_analyzed,
            limit: limit_val,
            offset: offset_val,
            max_revision,
            branch_override,
            locales,
            distinct,
        })
    }

    /// Analyze CTEs (Common Table Expressions) from WITH clause
    pub(super) fn analyze_ctes(
        &mut self,
        with: &sqlparser::ast::With,
    ) -> Result<Vec<(String, Box<AnalyzedQuery>)>> {
        if with.recursive {
            return Err(AnalysisError::UnsupportedStatement(
                "RECURSIVE CTEs not yet supported".into(),
            ));
        }

        let mut analyzed_ctes = Vec::new();

        for cte in &with.cte_tables {
            let cte_name = cte.alias.name.value.clone();

            if self.cte_catalog.contains_key(&cte_name) {
                return Err(AnalysisError::UnsupportedStatement(format!(
                    "Duplicate CTE name: {}",
                    cte_name
                )));
            }

            let cte_query = self.analyze_query(&cte.query)?;
            let schema = self.infer_cte_schema(&cte_name, &cte_query)?;

            let cte_def = CteDefinition {
                name: cte_name.clone(),
                query: Box::new(cte_query.clone()),
                schema: schema.clone(),
            };
            self.cte_catalog.insert(cte_name.clone(), cte_def);

            analyzed_ctes.push((cte_name, Box::new(cte_query)));
        }

        Ok(analyzed_ctes)
    }

    /// Infer schema from a CTE's projection
    pub(super) fn infer_cte_schema(
        &self,
        cte_name: &str,
        query: &AnalyzedQuery,
    ) -> Result<TableDef> {
        let mut columns = Vec::new();

        for (idx, (expr, alias)) in query.projection.iter().enumerate() {
            let col_name = if let Some(alias) = alias {
                alias.clone()
            } else {
                match &expr.expr {
                    Expr::Column { column, .. } => column.clone(),
                    _ => format!("col{}", idx),
                }
            };

            columns.push(ColumnDef {
                name: col_name,
                data_type: expr.data_type.clone(),
                nullable: true,
                generated: None,
            });
        }

        Ok(TableDef {
            name: cte_name.to_string(),
            columns,
            primary_key: Vec::new(),
            indexes: Vec::new(),
        })
    }

    /// Analyze projection (SELECT list)
    pub(super) fn analyze_projection(
        &self,
        projection: &[SelectItem],
    ) -> Result<Vec<(TypedExpr, Option<String>)>> {
        let mut result = Vec::new();

        for item in projection {
            match item {
                SelectItem::UnnamedExpr(expr) => {
                    let typed_expr = self.analyze_expr(expr)?;
                    result.push((typed_expr, None));
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    let typed_expr = self.analyze_expr(expr)?;
                    result.push((typed_expr, Some(alias.value.clone())));
                }
                SelectItem::Wildcard(_) => {
                    if self.current_tables.is_empty() {
                        return Err(AnalysisError::UnsupportedStatement(
                            "SELECT * requires a FROM clause".into(),
                        ));
                    }
                    // Expand * to all columns from all current tables
                    for table_ref in &self.current_tables {
                        self.expand_wildcard_for_table(table_ref, &mut result)?;
                    }
                }
                SelectItem::QualifiedWildcard(kind, _) => {
                    let obj_name = match kind {
                        SelectItemQualifiedWildcardKind::ObjectName(name) => name,
                        _ => {
                            return Err(AnalysisError::UnsupportedExpression(
                                "Unsupported qualified wildcard".into(),
                            ))
                        }
                    };

                    let table_or_alias = obj_name
                        .0
                        .iter()
                        .filter_map(|part| part.as_ident().map(|i| i.value.as_str()))
                        .collect::<Vec<_>>()
                        .join(".");

                    // Find the table reference
                    let table_ref = self
                        .current_tables
                        .iter()
                        .find(|t| t.name() == table_or_alias)
                        .ok_or_else(|| AnalysisError::TableNotFound(table_or_alias.clone()))?;

                    self.expand_wildcard_for_table(table_ref, &mut result)?;
                }
            }
        }

        Ok(result)
    }

    /// Expand wildcard (*) for a specific table
    pub(super) fn expand_wildcard_for_table(
        &self,
        table_ref: &super::types::TableRef,
        result: &mut Vec<(TypedExpr, Option<String>)>,
    ) -> Result<()> {
        // For table functions (like GRAPH_TABLE), use their dynamic schema
        if let Some(tf) = &table_ref.table_function {
            for col in &tf.schema.columns {
                let typed_expr = TypedExpr::column(
                    table_ref.name().to_string(),
                    col.name.clone(),
                    col.data_type.clone(),
                );
                result.push((typed_expr, Some(col.name.clone())));
            }
        } else if let Some(sq) = &table_ref.subquery {
            // For subqueries, use their schema
            for col in &sq.schema.columns {
                let typed_expr = TypedExpr::column(
                    table_ref.name().to_string(),
                    col.name.clone(),
                    col.data_type.clone(),
                );
                result.push((typed_expr, Some(col.name.clone())));
            }
        } else {
            // For regular tables, look up in catalog
            let table = self
                .get_table_def(&table_ref.table)?
                .ok_or_else(|| AnalysisError::TableNotFound(table_ref.table.clone()))?;

            for col in &table.columns {
                let typed_expr = TypedExpr::column(
                    table_ref.name().to_string(),
                    col.name.clone(),
                    col.data_type.clone(),
                );
                result.push((typed_expr, Some(col.name.clone())));
            }
        }

        Ok(())
    }
}
