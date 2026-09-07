//! Subquery expression analysis
//!
//! - `expr IN (SELECT ...)` / `expr = ANY (SELECT ...)` → [`Expr::InSubquery`]
//!   (planned as a semi-join)
//! - `EXISTS (SELECT ...)` → [`Expr::Exists`]
//! - `(SELECT ...)` as a value → [`Expr::ScalarSubquery`]
//! - `expr <op> ANY|ALL (SELECT ...)` → [`Expr::QuantifiedSubquery`]
//!
//! Every subquery is analysed in a NESTED scope that sees the outer CTEs but
//! none of the outer tables, so a correlated reference fails here with an
//! error naming the limitation instead of leaking through as a NULL column.

use crate::analyzer::{
    error::AnalysisError,
    semantic::{AnalyzedQuery, AnalyzerContext, Result},
    typed_expr::{BinaryOperator, Expr, TypedExpr},
    types::DataType,
};
use sqlparser::ast::Expr as SqlExpr;

impl<'a> AnalyzerContext<'a> {
    /// Analyse a subquery body in its own scope, carrying the outer CTEs into
    /// its `ctes` list so the plan for the subquery is self-contained.
    pub(in crate::analyzer::semantic) fn analyze_nested_query(
        &self,
        subquery: &sqlparser::ast::Query,
        what: &str,
    ) -> Result<AnalyzedQuery> {
        let mut subquery_ctx = self.nested_scope();

        let mut analyzed = match subquery_ctx.analyze_query(subquery) {
            Ok(q) => q,
            Err(AnalysisError::ColumnNotFound { table, column })
                if self.outer_has_column(&column) =>
            {
                return Err(AnalysisError::UnsupportedExpression(format!(
                    "{what}: column '{column}' is not visible inside the subquery (tables: {table}). \
                     Correlated subqueries that reference the outer query are not supported; \
                     rewrite as a JOIN or an IN (SELECT ...) predicate"
                )));
            }
            Err(e) => return Err(e),
        };

        for (cte_name, cte_def) in &self.cte_catalog {
            if !analyzed.ctes.iter().any(|(name, _)| name == cte_name) {
                analyzed
                    .ctes
                    .push((cte_name.clone(), cte_def.query.clone()));
            }
        }

        Ok(analyzed)
    }

    /// Does any table in the enclosing scope resolve `column`? Used only to
    /// improve the error for a correlated reference.
    fn outer_has_column(&self, column: &str) -> bool {
        self.current_tables.iter().any(|t| {
            if let Some(sq) = &t.subquery {
                return sq.schema.get_column(column).is_some();
            }
            match self.get_table_def(&t.table) {
                Ok(Some(def)) => def.get_column(column).is_some() || def.columns.is_empty(),
                _ => false,
            }
        })
    }

    fn single_column_type(query: &AnalyzedQuery, what: &str) -> Result<DataType> {
        if query.projection.len() != 1 {
            return Err(AnalysisError::UnsupportedExpression(format!(
                "{what} must return exactly one column, got {}",
                query.projection.len()
            )));
        }
        Ok(query.projection[0].0.data_type.clone())
    }

    /// Analyze IN subquery expression
    pub(in crate::analyzer::semantic) fn analyze_in_subquery(
        &self,
        expr: &SqlExpr,
        subquery: &sqlparser::ast::Query,
        negated: bool,
    ) -> Result<TypedExpr> {
        let typed_expr = self.analyze_expr(expr)?;
        let analyzed_subquery = self.analyze_nested_query(subquery, "IN (subquery)")?;
        let subquery_type = Self::single_column_type(&analyzed_subquery, "IN subquery")?;

        if typed_expr.data_type.common_type(&subquery_type).is_none() {
            return Err(AnalysisError::TypeMismatch {
                expected: typed_expr.data_type.to_string(),
                actual: subquery_type.to_string(),
            });
        }

        Ok(TypedExpr::new(
            Expr::InSubquery {
                expr: Box::new(typed_expr),
                subquery: Box::new(analyzed_subquery),
                subquery_type,
                negated,
            },
            DataType::Boolean,
        ))
    }

    /// `[NOT] EXISTS (SELECT ...)`
    pub(in crate::analyzer::semantic) fn analyze_exists(
        &self,
        subquery: &sqlparser::ast::Query,
        negated: bool,
    ) -> Result<TypedExpr> {
        let analyzed = self.analyze_nested_query(subquery, "EXISTS (subquery)")?;
        Ok(TypedExpr::new(
            Expr::Exists {
                subquery: Box::new(analyzed),
                negated,
            },
            DataType::Boolean,
        ))
    }

    /// `(SELECT ...)` used as a value.
    pub(in crate::analyzer::semantic) fn analyze_scalar_subquery(
        &self,
        subquery: &sqlparser::ast::Query,
    ) -> Result<TypedExpr> {
        let analyzed = self.analyze_nested_query(subquery, "scalar subquery")?;
        let data_type = Self::single_column_type(&analyzed, "a scalar subquery")?;
        Ok(TypedExpr::new(
            Expr::ScalarSubquery {
                subquery: Box::new(analyzed),
            },
            DataType::Nullable(Box::new(data_type.base_type().clone())),
        ))
    }

    /// `left <op> ANY|SOME|ALL (right)` where `right` is an array value or a
    /// subquery.
    pub(in crate::analyzer::semantic) fn analyze_quantified(
        &self,
        left: &SqlExpr,
        compare_op: &sqlparser::ast::BinaryOperator,
        right: &SqlExpr,
        all: bool,
    ) -> Result<TypedExpr> {
        let op = self.convert_binary_op(compare_op)?;
        if !matches!(
            op,
            BinaryOperator::Eq
                | BinaryOperator::NotEq
                | BinaryOperator::Lt
                | BinaryOperator::LtEq
                | BinaryOperator::Gt
                | BinaryOperator::GtEq
        ) {
            return Err(AnalysisError::UnsupportedExpression(format!(
                "{} with operator {compare_op}: only comparison operators (=, <>, <, <=, >, >=) are supported",
                if all { "ALL" } else { "ANY" }
            )));
        }

        if let SqlExpr::Subquery(query) = right {
            // `= ANY (subquery)` is `IN`, `<> ALL (subquery)` is `NOT IN`:
            // route to the semi-join plan instead of materialising.
            match (op, all) {
                (BinaryOperator::Eq, false) => {
                    return self.analyze_in_subquery(left, query, false);
                }
                (BinaryOperator::NotEq, true) => {
                    return self.analyze_in_subquery(left, query, true);
                }
                _ => {}
            }
            let typed_left = self.analyze_expr(left)?;
            let analyzed = self.analyze_nested_query(query, "ANY/ALL (subquery)")?;
            let sub_type = Self::single_column_type(&analyzed, "an ANY/ALL subquery")?;
            if typed_left.data_type.common_type(&sub_type).is_none() {
                return Err(AnalysisError::TypeMismatch {
                    expected: typed_left.data_type.to_string(),
                    actual: sub_type.to_string(),
                });
            }
            return Ok(TypedExpr::new(
                Expr::QuantifiedSubquery {
                    left: Box::new(typed_left),
                    op,
                    subquery: Box::new(analyzed),
                    all,
                },
                DataType::Boolean,
            ));
        }

        let typed_left = self.analyze_expr(left)?;
        let typed_right = self.analyze_array_value(right)?;
        Ok(TypedExpr::new(
            Expr::Quantified {
                left: Box::new(typed_left),
                op,
                right: Box::new(typed_right),
                all,
            },
            DataType::Boolean,
        ))
    }

    /// The array side of ANY/ALL: an `ARRAY[...]` of literals becomes a JSONB
    /// array literal; anything else (a `'[..]'` text parameter, a JSONB
    /// property, a `{a,b}` text) is analysed as-is and decoded at evaluation.
    fn analyze_array_value(&self, expr: &SqlExpr) -> Result<TypedExpr> {
        use crate::analyzer::typed_expr::Literal;
        if let SqlExpr::Array(array) = expr {
            let mut items = Vec::with_capacity(array.elem.len());
            for element in &array.elem {
                let typed = self.analyze_expr(element)?;
                let json = match &typed.expr {
                    Expr::Literal(lit) => literal_to_json(lit).ok_or_else(|| {
                        AnalysisError::UnsupportedExpression(format!(
                            "ARRAY element in ANY/ALL must be a literal, got {element}"
                        ))
                    })?,
                    _ => {
                        return Err(AnalysisError::UnsupportedExpression(format!(
                            "ARRAY element in ANY/ALL must be a literal, got {element}"
                        )))
                    }
                };
                items.push(json);
            }
            return Ok(TypedExpr::new(
                Expr::Literal(Literal::JsonB(serde_json::Value::Array(items))),
                DataType::JsonB,
            ));
        }
        let typed = self.analyze_expr(expr)?;
        match typed.data_type.base_type() {
            DataType::JsonB | DataType::Text | DataType::Unknown | DataType::Array(_) => Ok(typed),
            other => Err(AnalysisError::TypeMismatch {
                expected: "an array (ARRAY[...], JSONB array or JSON text)".into(),
                actual: other.to_string(),
            }),
        }
    }
}

/// Literal → JSON for building an ANY/ALL array. `None` for kinds that have
/// no JSON spelling worth comparing against (vectors, geometry, parameters).
fn literal_to_json(lit: &crate::analyzer::typed_expr::Literal) -> Option<serde_json::Value> {
    use crate::analyzer::typed_expr::Literal;
    use serde_json::Value;
    Some(match lit {
        Literal::Null => Value::Null,
        Literal::Boolean(b) => Value::Bool(*b),
        Literal::Int(i) => Value::from(*i),
        Literal::BigInt(i) => Value::from(*i),
        Literal::Double(d) => Value::from(*d),
        Literal::Text(s) | Literal::Uuid(s) | Literal::Path(s) => Value::String(s.clone()),
        Literal::JsonB(v) => v.clone(),
        Literal::Timestamp(ts) => Value::String(ts.to_rfc3339()),
        _ => return None,
    })
}
