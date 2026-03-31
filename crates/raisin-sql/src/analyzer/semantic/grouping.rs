//! GROUP BY, aggregate, and ORDER BY analysis
//!
//! This module handles the analysis of:
//! - GROUP BY clauses
//! - Aggregate functions (COUNT, SUM, AVG, MIN, MAX, ARRAY_AGG)
//! - ORDER BY clauses
//! - LIMIT and OFFSET clauses
//! - DISTINCT clauses

use super::equivalence::expressions_equivalent;
use super::types::{AnalyzedDistinct, OrderBySpec};
use super::{AnalyzerContext, Result};
use crate::analyzer::{
    error::AnalysisError,
    typed_expr::{Expr, TypedExpr},
    types::DataType,
};
use crate::logical_plan::operators::{AggregateExpr, AggregateFunction};
use sqlparser::ast::{GroupByExpr, OrderByExpr, Value, ValueWithSpan};

impl<'a> AnalyzerContext<'a> {
    /// Analyze GROUP BY expressions
    pub(super) fn analyze_group_by(&self, group_by: &GroupByExpr) -> Result<Vec<TypedExpr>> {
        match group_by {
            GroupByExpr::All(_) => Err(AnalysisError::UnsupportedStatement(
                "GROUP BY ALL not yet supported".into(),
            )),
            GroupByExpr::Expressions(exprs, modifiers) => {
                if !modifiers.is_empty() {
                    return Err(AnalysisError::UnsupportedStatement(
                        "GROUP BY modifiers (ROLLUP, CUBE, GROUPING SETS) not supported".into(),
                    ));
                }

                let mut result = Vec::new();
                for expr in exprs {
                    let typed_expr = self.analyze_expr(expr)?;
                    result.push(typed_expr);
                }
                Ok(result)
            }
        }
    }

    /// Extract aggregate function calls from an expression
    /// Returns Some(vec) if any aggregates found, None otherwise
    pub(super) fn extract_aggregates(
        &self,
        expr: &TypedExpr,
        alias: Option<&str>,
    ) -> Result<Option<Vec<AggregateExpr>>> {
        let mut aggregates = Vec::new();
        self.collect_aggregates(expr, alias, &mut aggregates)?;

        if aggregates.is_empty() {
            Ok(None)
        } else {
            Ok(Some(aggregates))
        }
    }

    /// Recursively collect aggregate function calls
    #[allow(clippy::only_used_in_recursion)]
    pub(super) fn collect_aggregates(
        &self,
        expr: &TypedExpr,
        alias: Option<&str>,
        aggregates: &mut Vec<AggregateExpr>,
    ) -> Result<()> {
        match &expr.expr {
            Expr::Function {
                name, args, filter, ..
            } => {
                // Check if this is an aggregate function
                let agg_func = match name.to_uppercase().as_str() {
                    "COUNT" => Some(AggregateFunction::Count),
                    "SUM" => Some(AggregateFunction::Sum),
                    "AVG" => Some(AggregateFunction::Avg),
                    "MIN" => Some(AggregateFunction::Min),
                    "MAX" => Some(AggregateFunction::Max),
                    "ARRAY_AGG" => Some(AggregateFunction::ArrayAgg),
                    _ => None,
                };

                if let Some(func) = agg_func {
                    // Determine the return type
                    let return_type = if matches!(func, AggregateFunction::ArrayAgg) {
                        // array_agg returns an array of the argument type
                        if args.len() == 1 {
                            DataType::Array(Box::new(args[0].data_type.clone()))
                        } else {
                            expr.data_type.clone()
                        }
                    } else {
                        expr.data_type.clone()
                    };

                    aggregates.push(AggregateExpr {
                        func,
                        args: args.clone(),
                        alias: alias.unwrap_or(name).to_string(),
                        return_type,
                        filter: filter.as_ref().map(|f| (**f).clone()),
                    });
                } else {
                    // Not an aggregate, recurse into arguments
                    for arg in args {
                        self.collect_aggregates(arg, None, aggregates)?;
                    }
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                self.collect_aggregates(left, None, aggregates)?;
                self.collect_aggregates(right, None, aggregates)?;
            }
            Expr::UnaryOp { expr, .. } => {
                self.collect_aggregates(expr, None, aggregates)?;
            }
            Expr::Cast { expr: inner, .. } => {
                self.collect_aggregates(inner, None, aggregates)?;
            }
            Expr::IsNull { expr } | Expr::IsNotNull { expr } => {
                self.collect_aggregates(expr, None, aggregates)?;
            }
            Expr::Between { expr, low, high } => {
                self.collect_aggregates(expr, None, aggregates)?;
                self.collect_aggregates(low, None, aggregates)?;
                self.collect_aggregates(high, None, aggregates)?;
            }
            Expr::InList { expr, list, .. } => {
                self.collect_aggregates(expr, None, aggregates)?;
                for item in list {
                    self.collect_aggregates(item, None, aggregates)?;
                }
            }
            Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
                self.collect_aggregates(expr, None, aggregates)?;
                self.collect_aggregates(pattern, None, aggregates)?;
            }
            Expr::JsonExtract { object, key }
            | Expr::JsonExtractText { object, key }
            | Expr::JsonContains {
                object,
                pattern: key,
            } => {
                self.collect_aggregates(object, None, aggregates)?;
                self.collect_aggregates(key, None, aggregates)?;
            }
            _ => {} // Literals, columns, etc. - no recursion needed
        }
        Ok(())
    }

    /// Validate GROUP BY usage
    /// - All non-aggregated columns in SELECT must appear in GROUP BY
    pub(super) fn validate_grouping(
        &self,
        projection: &[(TypedExpr, Option<String>)],
        group_by: &[TypedExpr],
        _aggregates: &[AggregateExpr],
    ) -> Result<()> {
        for (expr, _) in projection {
            if !self.is_valid_in_aggregate_query(expr, group_by)? {
                return Err(AnalysisError::ColumnNotInGroupBy(format!("{:?}", expr)));
            }
        }
        Ok(())
    }

    /// Check if an expression is valid in an aggregate query
    /// Valid if: (1) it's an aggregate function, or (2) it only references GROUP BY expressions
    pub(super) fn is_valid_in_aggregate_query(
        &self,
        expr: &TypedExpr,
        group_by: &[TypedExpr],
    ) -> Result<bool> {
        match &expr.expr {
            Expr::Function { name, args, .. } => {
                // Aggregate functions are valid
                let is_aggregate = matches!(
                    name.to_uppercase().as_str(),
                    "COUNT" | "SUM" | "AVG" | "MIN" | "MAX" | "ARRAY_AGG"
                );

                if is_aggregate {
                    return Ok(true);
                }

                // Check if this exact expression is in GROUP BY first
                for group_expr in group_by {
                    if expressions_equivalent(expr, group_expr) {
                        return Ok(true);
                    }
                }

                // Scalar functions are valid ONLY if all arguments are GROUP BY-independent
                for arg in args {
                    if !self.is_group_by_independent(arg)? {
                        return Ok(false);
                    }
                }

                Ok(true)
            }
            Expr::BinaryOp { left, right, .. } => {
                // Check if this exact expression is in GROUP BY first
                for group_expr in group_by {
                    if expressions_equivalent(expr, group_expr) {
                        return Ok(true);
                    }
                }

                let left_valid = self.is_valid_in_aggregate_query(left, group_by)?;
                let right_valid = self.is_valid_in_aggregate_query(right, group_by)?;
                Ok(left_valid && right_valid)
            }
            Expr::UnaryOp { expr: inner, .. } => {
                for group_expr in group_by {
                    if expressions_equivalent(expr, group_expr) {
                        return Ok(true);
                    }
                }
                self.is_valid_in_aggregate_query(inner, group_by)
            }
            Expr::Cast { expr: inner, .. } => {
                for group_expr in group_by {
                    if expressions_equivalent(expr, group_expr) {
                        return Ok(true);
                    }
                }
                self.is_valid_in_aggregate_query(inner, group_by)
            }
            Expr::Column { .. }
            | Expr::IsNull { .. }
            | Expr::IsNotNull { .. }
            | Expr::Between { .. }
            | Expr::InList { .. }
            | Expr::InSubquery { .. }
            | Expr::Like { .. }
            | Expr::ILike { .. }
            | Expr::JsonExtract { .. }
            | Expr::JsonExtractText { .. }
            | Expr::JsonContains { .. }
            | Expr::JsonKeyExists { .. }
            | Expr::JsonAnyKeyExists { .. }
            | Expr::JsonAllKeyExists { .. }
            | Expr::JsonExtractPath { .. }
            | Expr::JsonExtractPathText { .. }
            | Expr::JsonRemove { .. }
            | Expr::JsonRemoveAtPath { .. }
            | Expr::JsonPathMatch { .. }
            | Expr::JsonPathExists { .. } => {
                // Check if this expression is in GROUP BY
                for group_expr in group_by {
                    if expressions_equivalent(expr, group_expr) {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Expr::Window { .. } => {
                // Window functions are valid in aggregate contexts
                Ok(true)
            }
            Expr::Literal(_) => {
                // Literals are always valid
                Ok(true)
            }
            Expr::Case {
                conditions,
                else_expr,
            } => {
                for (cond, result) in conditions {
                    if !self.is_valid_in_aggregate_query(cond, group_by)? {
                        return Ok(false);
                    }
                    if !self.is_valid_in_aggregate_query(result, group_by)? {
                        return Ok(false);
                    }
                }
                if let Some(else_result) = else_expr {
                    if !self.is_valid_in_aggregate_query(else_result, group_by)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    /// Check if an expression is GROUP BY-independent
    #[allow(clippy::only_used_in_recursion)]
    pub(super) fn is_group_by_independent(&self, expr: &TypedExpr) -> Result<bool> {
        match &expr.expr {
            Expr::Literal(_) => Ok(true),
            Expr::Function { name, .. } => {
                let is_aggregate = matches!(
                    name.to_uppercase().as_str(),
                    "COUNT" | "SUM" | "AVG" | "MIN" | "MAX" | "ARRAY_AGG"
                );
                Ok(is_aggregate)
            }
            Expr::Window { .. } => Ok(true),
            Expr::BinaryOp { left, right, .. } => {
                Ok(self.is_group_by_independent(left)? && self.is_group_by_independent(right)?)
            }
            Expr::UnaryOp { expr: inner, .. } => self.is_group_by_independent(inner),
            Expr::Cast { expr: inner, .. } => self.is_group_by_independent(inner),
            _ => Ok(false),
        }
    }

    /// Analyze ORDER BY clause
    pub(super) fn analyze_order_by(
        &self,
        order_by: &[OrderByExpr],
        alias_map: &std::collections::HashMap<String, TypedExpr>,
    ) -> Result<Vec<OrderBySpec>> {
        let mut result = Vec::new();

        for order_expr in order_by {
            // Check if this is a simple identifier that matches an alias
            let typed_expr = if let sqlparser::ast::Expr::Identifier(ident) = &order_expr.expr {
                if let Some(aliased_expr) = alias_map.get(&ident.value) {
                    aliased_expr.clone()
                } else {
                    self.analyze_expr(&order_expr.expr)?
                }
            } else {
                self.analyze_expr(&order_expr.expr)?
            };

            let is_desc = order_expr.options.asc == Some(false);
            let nulls_first = order_expr.options.nulls_first;
            result.push(OrderBySpec::with_nulls(typed_expr, is_desc, nulls_first));
        }

        Ok(result)
    }

    /// Analyze LIMIT clause
    pub(super) fn analyze_limit(&self, expr: &sqlparser::ast::Expr) -> Result<usize> {
        match expr {
            sqlparser::ast::Expr::Value(ValueWithSpan {
                value: Value::Number(n, _),
                ..
            }) => {
                let val = n.parse::<i64>().map_err(|_| {
                    AnalysisError::InvalidLimit(format!("Invalid numeric value: {}", n))
                })?;

                if val < 0 {
                    return Err(AnalysisError::InvalidLimit(
                        "LIMIT must be non-negative".into(),
                    ));
                }

                Ok(val as usize)
            }
            _ => Err(AnalysisError::InvalidLimit(
                "LIMIT must be a positive integer".into(),
            )),
        }
    }

    /// Analyze OFFSET clause
    pub(super) fn analyze_offset(&self, expr: &sqlparser::ast::Expr) -> Result<usize> {
        match expr {
            sqlparser::ast::Expr::Value(ValueWithSpan {
                value: Value::Number(n, _),
                ..
            }) => {
                let val = n.parse::<i64>().map_err(|_| {
                    AnalysisError::InvalidOffset(format!("Invalid numeric value: {}", n))
                })?;

                if val < 0 {
                    return Err(AnalysisError::InvalidOffset(
                        "OFFSET must be non-negative".into(),
                    ));
                }

                Ok(val as usize)
            }
            _ => Err(AnalysisError::InvalidOffset(
                "OFFSET must be a positive integer".into(),
            )),
        }
    }

    /// Analyze DISTINCT clause
    pub(super) fn analyze_distinct(
        &mut self,
        distinct: &Option<sqlparser::ast::Distinct>,
        _projection: &[(TypedExpr, Option<String>)],
        order_by: &[sqlparser::ast::OrderByExpr],
    ) -> Result<Option<AnalyzedDistinct>> {
        use sqlparser::ast::Distinct;

        match distinct {
            None => Ok(None),
            Some(Distinct::Distinct) => Ok(Some(AnalyzedDistinct::All)),
            Some(Distinct::On(on_exprs)) => {
                // Analyze each DISTINCT ON expression
                let analyzed_on: Vec<TypedExpr> = on_exprs
                    .iter()
                    .map(|expr| self.analyze_expr(expr))
                    .collect::<Result<Vec<_>>>()?;

                // Validate PostgreSQL rule: DISTINCT ON expressions must match
                // leading ORDER BY expressions
                self.validate_distinct_on_order_by(&analyzed_on, order_by)?;

                Ok(Some(AnalyzedDistinct::On(analyzed_on)))
            }
        }
    }

    /// Validate that DISTINCT ON columns appear in ORDER BY's leading positions
    pub(super) fn validate_distinct_on_order_by(
        &self,
        distinct_on: &[TypedExpr],
        order_by: &[sqlparser::ast::OrderByExpr],
    ) -> Result<()> {
        if distinct_on.is_empty() || order_by.is_empty() {
            return Ok(());
        }

        if order_by.len() < distinct_on.len() {
            return Err(AnalysisError::UnsupportedStatement(
                "SELECT DISTINCT ON expressions must match leftmost ORDER BY expressions".into(),
            ));
        }

        // For now, we'll allow any ORDER BY with DISTINCT ON
        Ok(())
    }
}
