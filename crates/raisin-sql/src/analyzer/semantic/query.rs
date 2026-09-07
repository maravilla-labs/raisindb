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
    AnalyzedQuery, AnalyzedSetOperation, AnalyzedShow, AnalyzedStatement, CteDefinition,
    ExplainFormat, ExplainStatement, OrderBySpec, SetOperationKind,
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
                returning,
                ..
            } => self.analyze_update(table, assignments, selection.as_ref(), returning.as_deref()),
            Statement::Delete(delete) => self.analyze_delete(delete),
            Statement::ShowVariable { variable } => self.analyze_show_variable(variable),
            _ => Err(AnalysisError::UnsupportedStatement(
                describe_unsupported_statement(stmt),
            )),
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
                    target: Box::new(AnalyzedStatement::Query(analyzed_query)),
                    analyze,
                    format: explain_format,
                    verbose,
                }))
            }
            // EXPLAIN UPDATE / EXPLAIN DELETE — shows the row-matching plan
            // (point lookup vs bulk index scan) without executing the DML.
            Statement::Update {
                table,
                assignments,
                selection,
                ..
            } => {
                let analyzed = self.analyze_update(table, assignments, selection.as_ref(), None)?;
                Ok(AnalyzedStatement::Explain(ExplainStatement {
                    target: Box::new(analyzed),
                    analyze,
                    format: explain_format,
                    verbose,
                }))
            }
            Statement::Delete(delete) => {
                let analyzed = self.analyze_delete(delete)?;
                Ok(AnalyzedStatement::Explain(ExplainStatement {
                    target: Box::new(analyzed),
                    analyze,
                    format: explain_format,
                    verbose,
                }))
            }
            _ => Err(AnalysisError::UnsupportedStatement(
                "EXPLAIN only supports SELECT, UPDATE, and DELETE statements".into(),
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
            SetExpr::SetOperation { .. } | SetExpr::Query(_) => {
                self.analyze_set_operation_query(set_expr, order_by_exprs, limit_expr, offset_expr)?
            }
            SetExpr::Values(_) => {
                return Err(AnalysisError::UnsupportedStatement(
                    "VALUES as a standalone query is not supported (use SELECT)".into(),
                ))
            }
            other => {
                return Err(AnalysisError::UnsupportedStatement(format!(
                    "query body `{}`: only SELECT and UNION/INTERSECT/EXCEPT are supported",
                    truncate(&other.to_string())
                )))
            }
        };

        // Add CTEs to the analyzed query
        analyzed.ctes = ctes;
        Ok(analyzed)
    }

    /// Analyse one side of a set operation (or a nested set operation).
    /// Sides carry no ORDER BY / LIMIT of their own unless parenthesised as a
    /// full query.
    fn analyze_set_expr_side(&mut self, side: &SetExpr) -> Result<AnalyzedQuery> {
        match side {
            SetExpr::Select(select) => {
                let mut ctx = self.nested_scope();
                ctx.analyze_select(select, &[], None, None)
            }
            SetExpr::SetOperation { .. } => self.analyze_set_operation_query(side, &[], None, None),
            SetExpr::Query(query) => {
                let mut ctx = self.nested_scope();
                ctx.analyze_query(query)
            }
            other => Err(AnalysisError::UnsupportedStatement(format!(
                "set operation operand `{}`: only SELECT queries can be combined",
                truncate(&other.to_string())
            ))),
        }
    }

    /// `left UNION|INTERSECT|EXCEPT [ALL] right [ORDER BY ...] [LIMIT ...]`
    fn analyze_set_operation_query(
        &mut self,
        body: &SetExpr,
        order_by: &[OrderByExpr],
        limit: Option<&sqlparser::ast::Expr>,
        offset: Option<&sqlparser::ast::Expr>,
    ) -> Result<AnalyzedQuery> {
        use sqlparser::ast::{SetOperator, SetQuantifier};

        let (op, set_quantifier, left, right) = match body {
            SetExpr::SetOperation {
                op,
                set_quantifier,
                left,
                right,
            } => (op, set_quantifier, left, right),
            // A parenthesised query as the whole body: `(SELECT ...) ORDER BY`
            SetExpr::Query(query) => {
                let mut inner = self.nested_scope().analyze_query(query)?;
                if !order_by.is_empty() || limit.is_some() || offset.is_some() {
                    self.apply_outer_order_limit(&mut inner, order_by, limit, offset)?;
                }
                return Ok(inner);
            }
            _ => unreachable!("analyze_set_operation_query called on a non-set body"),
        };

        let kind = match op {
            SetOperator::Union => SetOperationKind::Union,
            SetOperator::Intersect => SetOperationKind::Intersect,
            SetOperator::Except | SetOperator::Minus => SetOperationKind::Except,
        };
        let all = match set_quantifier {
            SetQuantifier::All => true,
            SetQuantifier::Distinct | SetQuantifier::None => false,
            other => {
                return Err(AnalysisError::UnsupportedStatement(format!(
                    "{} {other}: BY NAME set operations are not supported",
                    kind.keyword()
                )))
            }
        };

        let left_q = self.analyze_set_expr_side(left)?;
        let right_q = self.analyze_set_expr_side(right)?;

        if left_q.projection.len() != right_q.projection.len() {
            return Err(AnalysisError::UnsupportedStatement(format!(
                "each {} query must have the same number of columns: left has {}, right has {}",
                kind.keyword(),
                left_q.projection.len(),
                right_q.projection.len()
            )));
        }
        for (i, ((l, _), (r, _))) in left_q
            .projection
            .iter()
            .zip(right_q.projection.iter())
            .enumerate()
        {
            let compatible = l.data_type.common_type(&r.data_type).is_some()
                || matches!(l.data_type.base_type(), DataType::Unknown)
                || matches!(r.data_type.base_type(), DataType::Unknown);
            if !compatible {
                return Err(AnalysisError::TypeMismatch {
                    expected: format!(
                        "{} column {} of type {}",
                        kind.keyword(),
                        i + 1,
                        l.data_type
                    ),
                    actual: r.data_type.to_string(),
                });
            }
        }

        let mut combined = AnalyzedQuery {
            ctes: Vec::new(),
            projection: left_q.projection.clone(),
            from: left_q.from.clone(),
            joins: Vec::new(),
            selection: None,
            group_by: Vec::new(),
            aggregates: Vec::new(),
            order_by: Vec::new(),
            limit: None,
            offset: None,
            max_revision: None,
            branch_override: None,
            locales: Vec::new(),
            distinct: None,
            having: None,
            set_operation: Some(Box::new(AnalyzedSetOperation {
                kind,
                all,
                left: left_q,
                right: right_q,
            })),
        };
        self.apply_outer_order_limit(&mut combined, order_by, limit, offset)?;
        Ok(combined)
    }

    /// ORDER BY / LIMIT / OFFSET over a combined result. ORDER BY may name
    /// output columns (by alias or by the left side's column name) only —
    /// there is no single input table to resolve arbitrary expressions
    /// against.
    fn apply_outer_order_limit(
        &mut self,
        query: &mut AnalyzedQuery,
        order_by: &[OrderByExpr],
        limit: Option<&sqlparser::ast::Expr>,
        offset: Option<&sqlparser::ast::Expr>,
    ) -> Result<()> {
        let output_columns: std::collections::HashMap<String, TypedExpr> = query
            .projection
            .iter()
            .map(|(expr, alias)| {
                let name = alias.clone().unwrap_or_else(|| output_column_name(expr));
                (
                    name.clone(),
                    TypedExpr::column(String::new(), name, expr.data_type.clone()),
                )
            })
            .collect();

        let mut specs = Vec::with_capacity(order_by.len());
        for item in order_by {
            let typed = match &item.expr {
                sqlparser::ast::Expr::Identifier(ident) => {
                    output_columns.get(&ident.value).cloned().ok_or_else(|| {
                        AnalysisError::ColumnNotFound {
                            table: "set operation result".into(),
                            column: ident.value.clone(),
                        }
                    })?
                }
                sqlparser::ast::Expr::Value(v) => {
                    // ORDER BY <ordinal>
                    let n = v.value.to_string().parse::<usize>().ok();
                    match n.and_then(|n| query.projection.get(n.wrapping_sub(1)).map(|p| (n, p))) {
                        Some((_, (expr, alias))) => {
                            let name = alias
                                .clone()
                                .unwrap_or_else(|| output_column_name(expr));
                            TypedExpr::column(String::new(), name, expr.data_type.clone())
                        }
                        None => {
                            return Err(AnalysisError::UnsupportedStatement(format!(
                                "ORDER BY position {} is not in the select list",
                                v.value
                            )))
                        }
                    }
                }
                other => {
                    return Err(AnalysisError::UnsupportedStatement(format!(
                        "ORDER BY `{other}` over a set operation: only output column names or positions are allowed"
                    )))
                }
            };
            let is_desc = item.options.asc == Some(false);
            specs.push(OrderBySpec::with_nulls(
                typed,
                is_desc,
                item.options.nulls_first,
            ));
        }
        query.order_by = specs;
        query.limit = match limit {
            Some(e) => Some(self.analyze_limit(e)?),
            None => None,
        };
        query.offset = match offset {
            Some(e) => Some(self.analyze_offset(e)?),
            None => None,
        };
        Ok(())
    }

    /// Analyze a SELECT statement
    pub(super) fn analyze_select(
        &mut self,
        select: &Select,
        order_by: &[OrderByExpr],
        limit: Option<&sqlparser::ast::Expr>,
        offset: Option<&sqlparser::ast::Expr>,
    ) -> Result<AnalyzedQuery> {
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

        // Build alias map for HAVING / ORDER BY resolution
        let alias_map: std::collections::HashMap<String, TypedExpr> = projection
            .iter()
            .filter_map(|(expr, alias)| alias.as_ref().map(|a| (a.clone(), expr.clone())))
            .collect();

        // Analyze HAVING: aliases resolve to their select-list expression;
        // aggregates it introduces are added to the aggregate list so the
        // grouping operator computes them even when they are not projected.
        let having = if let Some(having_expr) = &select.having {
            self.select_aliases = alias_map.clone();
            let typed = self.analyze_expr(having_expr);
            self.select_aliases.clear();
            let typed = typed?;
            if !matches!(typed.data_type.base_type(), DataType::Boolean) {
                return Err(AnalysisError::TypeMismatch {
                    expected: "BOOLEAN (HAVING)".into(),
                    actual: typed.data_type.to_string(),
                });
            }
            if let Some(agg_exprs) = self.extract_aggregates(&typed, None)? {
                for agg in agg_exprs {
                    let dup = aggregates.iter().any(
                        |a: &crate::logical_plan::operators::AggregateExpr| {
                            a.func == agg.func
                                && a.args.len() == agg.args.len()
                                && a.args
                                    .iter()
                                    .zip(agg.args.iter())
                                    .all(|(x, y)| super::equivalence::expressions_equivalent(x, y))
                                && a.filter.is_none() == agg.filter.is_none()
                        },
                    );
                    if !dup {
                        aggregates.push(agg);
                    }
                }
                has_aggregates = true;
            }
            if !self.is_valid_in_aggregate_query(&typed, &group_by)? {
                return Err(AnalysisError::ColumnNotInGroupBy(format!(
                    "HAVING references `{}`, which is neither grouped nor aggregated",
                    having_expr
                )));
            }
            Some(typed)
        } else {
            None
        };

        // Validate GROUP BY usage
        if has_aggregates || !group_by.is_empty() {
            self.validate_grouping(&projection, &group_by, &aggregates)?;
        }

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
            having,
            set_operation: None,
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
                // Opt-in pseudo-columns (`__distance`, `__matched_path`) are
                // selectable by name but never expanded by `*` — they are NULL on
                // every non-spatial access path, and expanding them would add two
                // always-NULL columns to every `SELECT *` in the system.
                if col
                    .generated
                    .as_ref()
                    .is_some_and(|g| g.hidden_from_wildcard())
                {
                    continue;
                }
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

/// The name a projection item gets in the output row when it has no alias.
/// Must agree with the plan builder's `derive_column_name`.
fn output_column_name(expr: &TypedExpr) -> String {
    match &expr.expr {
        Expr::Column { column, .. } => column.clone(),
        Expr::Function { name, .. } => name.to_lowercase(),
        Expr::Cast { expr, .. } => output_column_name(expr),
        _ => "?column?".to_string(),
    }
}

fn truncate(text: &str) -> String {
    if text.len() > 120 {
        format!("{}...", &text[..117])
    } else {
        text.to_string()
    }
}

/// Name the statement kind instead of dumping the AST.
fn describe_unsupported_statement(stmt: &Statement) -> String {
    let text = stmt.to_string();
    let keyword: String = text
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase();
    format!(
        "{keyword} statement is not supported: `{}`",
        truncate(&text)
    )
}
