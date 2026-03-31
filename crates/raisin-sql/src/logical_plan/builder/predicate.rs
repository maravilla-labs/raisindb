//! Predicate analysis helpers for the plan builder.

use super::PlanBuilder;
use crate::logical_plan::{
    error::{PlanError, Result},
    operators::LogicalPlan,
};

impl<'a> PlanBuilder<'a> {
    /// Extract all table references from an expression
    pub(crate) fn extract_table_references(
        expr: &crate::analyzer::TypedExpr,
    ) -> std::collections::HashSet<String> {
        use crate::analyzer::Expr;
        use std::collections::HashSet;

        let mut tables = HashSet::new();

        match &expr.expr {
            Expr::Column { table, .. } => {
                if !table.is_empty() {
                    tables.insert(table.clone());
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                tables.extend(Self::extract_table_references(left));
                tables.extend(Self::extract_table_references(right));
            }
            Expr::UnaryOp { expr: inner, .. } => {
                tables.extend(Self::extract_table_references(inner));
            }
            Expr::Function { args, filter, .. } => {
                for arg in args {
                    tables.extend(Self::extract_table_references(arg));
                }
                if let Some(f) = filter {
                    tables.extend(Self::extract_table_references(f));
                }
            }
            Expr::Cast { expr: inner, .. } => {
                tables.extend(Self::extract_table_references(inner));
            }
            Expr::IsNull { expr: inner } | Expr::IsNotNull { expr: inner } => {
                tables.extend(Self::extract_table_references(inner));
            }
            Expr::Between { expr, low, high } => {
                tables.extend(Self::extract_table_references(expr));
                tables.extend(Self::extract_table_references(low));
                tables.extend(Self::extract_table_references(high));
            }
            Expr::InList { expr, list, .. } => {
                tables.extend(Self::extract_table_references(expr));
                for item in list {
                    tables.extend(Self::extract_table_references(item));
                }
            }
            Expr::InSubquery { expr, .. } => {
                // Only extract from the left expression, subquery has its own scope
                tables.extend(Self::extract_table_references(expr));
            }
            Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
                tables.extend(Self::extract_table_references(expr));
                tables.extend(Self::extract_table_references(pattern));
            }
            Expr::JsonExtract { object, key }
            | Expr::JsonExtractText { object, key }
            | Expr::JsonContains {
                object,
                pattern: key,
            }
            | Expr::JsonKeyExists { object, key }
            | Expr::JsonAnyKeyExists { object, keys: key }
            | Expr::JsonAllKeyExists { object, keys: key }
            | Expr::JsonExtractPath { object, path: key }
            | Expr::JsonExtractPathText { object, path: key }
            | Expr::JsonRemove { object, key }
            | Expr::JsonRemoveAtPath { object, path: key }
            | Expr::JsonPathMatch { object, path: key }
            | Expr::JsonPathExists { object, path: key } => {
                tables.extend(Self::extract_table_references(object));
                tables.extend(Self::extract_table_references(key));
            }
            Expr::Case {
                conditions,
                else_expr,
            } => {
                for (cond, result) in conditions {
                    tables.extend(Self::extract_table_references(cond));
                    tables.extend(Self::extract_table_references(result));
                }
                if let Some(else_expr_box) = else_expr {
                    tables.extend(Self::extract_table_references(else_expr_box));
                }
            }
            Expr::Literal(_) => {}
            Expr::Window {
                partition_by,
                order_by,
                ..
            } => {
                for expr in partition_by {
                    tables.extend(Self::extract_table_references(expr));
                }
                for (expr, _) in order_by {
                    tables.extend(Self::extract_table_references(expr));
                }
            }
        }

        tables
    }

    /// Split a predicate into conjunctions (AND-separated parts)
    pub(crate) fn split_conjunctions(
        expr: &crate::analyzer::TypedExpr,
    ) -> Vec<crate::analyzer::TypedExpr> {
        use crate::analyzer::{BinaryOperator, Expr};

        match &expr.expr {
            Expr::BinaryOp {
                left,
                op: BinaryOperator::And,
                right,
            } => {
                let mut result = Self::split_conjunctions(left);
                result.extend(Self::split_conjunctions(right));
                result
            }
            _ => vec![expr.clone()],
        }
    }

    /// Combine multiple predicates with AND
    pub(crate) fn combine_with_and(
        predicates: Vec<crate::analyzer::TypedExpr>,
    ) -> Option<crate::analyzer::TypedExpr> {
        use crate::analyzer::{BinaryOperator, DataType, Expr, TypedExpr};

        if predicates.is_empty() {
            return None;
        }

        if predicates.len() == 1 {
            return Some(predicates.into_iter().next().unwrap());
        }

        let mut iter = predicates.into_iter();
        let mut result = iter.next().unwrap();

        for pred in iter {
            result = TypedExpr::new(
                Expr::BinaryOp {
                    left: Box::new(result),
                    op: BinaryOperator::And,
                    right: Box::new(pred),
                },
                DataType::Boolean,
            );
        }

        Some(result)
    }

    /// Extract InSubquery expressions from a list of predicates
    /// Returns (in_subquery_predicates, remaining_predicates)
    pub(crate) fn extract_in_subquery_predicates(
        predicates: Vec<crate::analyzer::TypedExpr>,
    ) -> (
        Vec<crate::analyzer::TypedExpr>,
        Vec<crate::analyzer::TypedExpr>,
    ) {
        use crate::analyzer::Expr;

        let mut in_subquery_predicates = Vec::new();
        let mut remaining_predicates = Vec::new();

        for pred in predicates {
            if matches!(pred.expr, Expr::InSubquery { .. }) {
                in_subquery_predicates.push(pred);
            } else {
                remaining_predicates.push(pred);
            }
        }

        (in_subquery_predicates, remaining_predicates)
    }

    /// Build a SemiJoin from an InSubquery expression
    /// Returns the SemiJoin plan node
    pub(crate) fn build_semi_join_from_in_subquery(
        &self,
        plan: LogicalPlan,
        in_subquery: &crate::analyzer::TypedExpr,
    ) -> Result<LogicalPlan> {
        use crate::analyzer::Expr;

        if let Expr::InSubquery {
            expr,
            subquery,
            negated,
            ..
        } = &in_subquery.expr
        {
            // Build the subquery plan
            let subquery_plan = self.build_query(subquery)?;

            // The right key is a column reference to the first (and only) projection column
            // from the subquery
            let right_key = if !subquery.projection.is_empty() {
                let (proj_expr, proj_alias) = &subquery.projection[0];
                let col_name = proj_alias
                    .clone()
                    .unwrap_or_else(|| Self::derive_column_name(proj_expr));

                crate::analyzer::TypedExpr::new(
                    Expr::Column {
                        table: String::new(),
                        column: col_name,
                    },
                    proj_expr.data_type.clone(),
                )
            } else {
                return Err(PlanError::InvalidPlan(
                    "IN subquery must have at least one projection column".to_string(),
                ));
            };

            Ok(LogicalPlan::SemiJoin {
                left: Box::new(plan),
                right: Box::new(subquery_plan),
                left_key: (**expr).clone(),
                right_key,
                anti: *negated,
            })
        } else {
            Err(PlanError::InvalidPlan(
                "Expected InSubquery expression".to_string(),
            ))
        }
    }

    /// Split predicates by the table they reference
    /// Returns (table_predicates, remaining_predicates)
    /// where table_predicates maps table name/alias to predicates that only reference that table
    pub(crate) fn split_predicates_by_table(
        predicate: &crate::analyzer::TypedExpr,
        table_refs: &[crate::analyzer::TableRef],
    ) -> (
        std::collections::HashMap<String, Vec<crate::analyzer::TypedExpr>>,
        Vec<crate::analyzer::TypedExpr>,
    ) {
        use crate::analyzer::Expr;
        use std::collections::HashMap;

        let mut table_predicates: HashMap<String, Vec<crate::analyzer::TypedExpr>> = HashMap::new();
        let mut remaining_predicates = Vec::new();

        // Build set of table names/aliases
        let table_names: std::collections::HashSet<String> = table_refs
            .iter()
            .map(|tr| tr.alias.clone().unwrap_or_else(|| tr.table.clone()))
            .collect();

        // Split predicate into individual conjunctions
        let conjunctions = Self::split_conjunctions(predicate);

        for conj in conjunctions {
            // InSubquery predicates should NOT be pushed down to scans -
            // they need to be transformed into SemiJoin operators later.
            // Always put them in remaining_predicates.
            if matches!(conj.expr, Expr::InSubquery { .. }) {
                remaining_predicates.push(conj);
                continue;
            }

            let referenced_tables = Self::extract_table_references(&conj);

            // Check if this predicate references only one table
            if referenced_tables.len() == 1 {
                let table = referenced_tables.iter().next().unwrap();

                // Verify this table is in our FROM/JOIN list
                if table_names.contains(table) {
                    table_predicates
                        .entry(table.clone())
                        .or_default()
                        .push(conj);
                    continue;
                }
            }

            // Predicate references multiple tables or no tables - can't push down
            remaining_predicates.push(conj);
        }

        (table_predicates, remaining_predicates)
    }
}
