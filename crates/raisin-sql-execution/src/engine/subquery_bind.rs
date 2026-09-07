//! Bind uncorrelated subqueries to literals before planning.
//!
//! `EXISTS (…)`, scalar `(SELECT …)` and `x <op> ANY|ALL (SELECT …)` are
//! analysed into expression nodes that carry the whole sub-`AnalyzedQuery`.
//! None of them can be evaluated row-by-row by the synchronous expression
//! evaluator, and none of them may reference the outer row (the analyzer
//! rejects that), so their value is a CONSTANT for the statement. This pass
//! runs each one exactly once, through the same engine entry point as any
//! other query, and substitutes the result:
//!
//! | node                 | becomes                                   |
//! |----------------------|-------------------------------------------|
//! | `Exists`             | `Literal::Boolean`                        |
//! | `ScalarSubquery`     | the single column of the single row, or NULL |
//! | `QuantifiedSubquery` | `Quantified` over a `Literal::JsonB` array |
//!
//! `INSERT … SELECT` is bound the same way: the source query is executed and
//! each result row becomes a literal VALUES row.
//!
//! Doing this BEFORE planning (rather than at execution) means the planner
//! sees a literal and can still pick an index for `WHERE path = (SELECT …)`.
//! Subqueries nest: a subquery's own subqueries are bound first.

use super::QueryEngine;
use crate::physical_plan::executor::Row;
use crate::physical_plan::types::from_property_value;
use futures::StreamExt;
use raisin_error::Error;
use raisin_sql::analyzer::{
    AnalyzedDistinct, AnalyzedQuery, AnalyzedStatement, Expr, Literal, TypedExpr,
};
use raisin_storage::Storage;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;

/// Which literal shape a pending subquery must produce.
#[derive(Debug, Clone, Copy)]
enum BindKind {
    Exists,
    Scalar,
    Array,
}

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Resolve every EXISTS / scalar / ANY-ALL subquery and INSERT…SELECT
    /// source in `stmt`. A statement without any of them is returned as-is,
    /// without cloning anything.
    pub(crate) async fn bind_subqueries(
        &self,
        mut stmt: AnalyzedStatement,
    ) -> Result<AnalyzedStatement, Error> {
        if !statement_needs_binding(&stmt) {
            return Ok(stmt);
        }
        self.bind_statement(&mut stmt).await?;
        Ok(stmt)
    }

    fn bind_statement<'a>(&'a self, stmt: &'a mut AnalyzedStatement) -> BoxFut<'a, ()> {
        Box::pin(async move {
            match stmt {
                AnalyzedStatement::Query(q) => self.bind_query(q).await,
                AnalyzedStatement::Explain(explain) => {
                    self.bind_statement(&mut explain.target).await
                }
                AnalyzedStatement::Insert(insert) => {
                    if let Some(source) = insert.source.take() {
                        let mut source = *source;
                        self.bind_query(&mut source).await?;
                        let rows = self.run_subquery(source).await?;
                        insert.values = rows
                            .into_iter()
                            .map(|row| {
                                row.columns
                                    .values()
                                    .map(|v| {
                                        from_property_value(v)
                                            .map(TypedExpr::literal)
                                            .map_err(Error::Validation)
                                    })
                                    .collect::<Result<Vec<_>, _>>()
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                    }
                    let mut slots: Vec<&mut TypedExpr> = Vec::new();
                    for row in &mut insert.values {
                        slots.extend(row.iter_mut());
                    }
                    if let Some(returning) = &mut insert.returning {
                        slots.extend(returning.iter_mut().map(|(e, _)| e));
                    }
                    self.bind_slots(slots).await
                }
                AnalyzedStatement::Update(update) => {
                    let mut slots: Vec<&mut TypedExpr> =
                        update.assignments.iter_mut().map(|(_, e)| e).collect();
                    slots.extend(update.filter.iter_mut());
                    if let Some(returning) = &mut update.returning {
                        slots.extend(returning.iter_mut().map(|(e, _)| e));
                    }
                    self.bind_slots(slots).await
                }
                AnalyzedStatement::Delete(delete) => {
                    let mut slots: Vec<&mut TypedExpr> = delete.filter.iter_mut().collect();
                    if let Some(returning) = &mut delete.returning {
                        slots.extend(returning.iter_mut().map(|(e, _)| e));
                    }
                    self.bind_slots(slots).await
                }
                _ => Ok(()),
            }
        })
    }

    /// Bind nested query scopes first (CTEs, set-operation sides, derived
    /// tables), then this query's own expression slots.
    fn bind_query<'a>(&'a self, query: &'a mut AnalyzedQuery) -> BoxFut<'a, ()> {
        Box::pin(async move {
            for (_, cte) in &mut query.ctes {
                self.bind_query(cte).await?;
            }
            if let Some(set_op) = &mut query.set_operation {
                self.bind_query(&mut set_op.left).await?;
                self.bind_query(&mut set_op.right).await?;
            }
            for table in &mut query.from {
                if let Some(sub) = &mut table.subquery {
                    self.bind_query(&mut sub.query).await?;
                }
            }
            for join in &mut query.joins {
                if let Some(sub) = &mut join.right_table.subquery {
                    self.bind_query(&mut sub.query).await?;
                }
            }
            self.bind_slots(query_expr_slots(query)).await
        })
    }

    /// Two-phase: collect every pending subquery in a fixed pre-order,
    /// execute them, then substitute in the same order.
    async fn bind_slots(&self, mut slots: Vec<&mut TypedExpr>) -> Result<(), Error> {
        let mut pending: Vec<(BindKind, AnalyzedQuery)> = Vec::new();
        for slot in slots.iter() {
            slot.walk(&mut |e| match &e.expr {
                Expr::Exists { subquery, .. } => {
                    pending.push((BindKind::Exists, (**subquery).clone()))
                }
                Expr::ScalarSubquery { subquery } => {
                    pending.push((BindKind::Scalar, (**subquery).clone()))
                }
                Expr::QuantifiedSubquery { subquery, .. } => {
                    pending.push((BindKind::Array, (**subquery).clone()))
                }
                _ => {}
            });
        }
        if pending.is_empty() {
            return Ok(());
        }

        let mut results: VecDeque<Literal> = VecDeque::with_capacity(pending.len());
        for (kind, mut subquery) in pending {
            self.bind_query(&mut subquery).await?;
            if matches!(kind, BindKind::Exists) && subquery.limit.map_or(true, |l| l > 1) {
                // EXISTS only needs to know whether ONE row comes back.
                subquery.limit = Some(1);
            }
            let rows = self.run_subquery(subquery).await?;
            results.push_back(literal_for(kind, rows)?);
        }

        for slot in slots.iter_mut() {
            substitute(slot, &mut results);
        }
        debug_assert!(results.is_empty(), "subquery results left unconsumed");
        Ok(())
    }

    async fn run_subquery(&self, query: AnalyzedQuery) -> Result<Vec<Row>, Error> {
        let stmt = AnalyzedStatement::Query(query);
        let mut stream = self.execute_analyzed_statement(&stmt).await?;
        let mut rows = Vec::new();
        while let Some(row) = stream.next().await {
            rows.push(row?);
        }
        Ok(rows)
    }
}

fn statement_needs_binding(stmt: &AnalyzedStatement) -> bool {
    match stmt {
        AnalyzedStatement::Query(q) => query_needs_binding(q),
        AnalyzedStatement::Explain(e) => statement_needs_binding(&e.target),
        AnalyzedStatement::Insert(i) => {
            i.source.is_some()
                || i.values
                    .iter()
                    .flatten()
                    .any(TypedExpr::has_unbound_subquery)
                || returning_needs_binding(i.returning.as_deref())
        }
        AnalyzedStatement::Update(u) => {
            u.assignments.iter().any(|(_, e)| e.has_unbound_subquery())
                || u.filter
                    .as_ref()
                    .is_some_and(TypedExpr::has_unbound_subquery)
                || returning_needs_binding(u.returning.as_deref())
        }
        AnalyzedStatement::Delete(d) => {
            d.filter
                .as_ref()
                .is_some_and(TypedExpr::has_unbound_subquery)
                || returning_needs_binding(d.returning.as_deref())
        }
        _ => false,
    }
}

fn returning_needs_binding(items: Option<&[(TypedExpr, Option<String>)]>) -> bool {
    items.is_some_and(|items| items.iter().any(|(e, _)| e.has_unbound_subquery()))
}

fn query_needs_binding(q: &AnalyzedQuery) -> bool {
    let exprs_pending = q.projection.iter().any(|(e, _)| e.has_unbound_subquery())
        || q.selection
            .as_ref()
            .is_some_and(TypedExpr::has_unbound_subquery)
        || q.having
            .as_ref()
            .is_some_and(TypedExpr::has_unbound_subquery)
        || q.group_by.iter().any(TypedExpr::has_unbound_subquery)
        || q.order_by.iter().any(|o| o.expr.has_unbound_subquery())
        || q.joins.iter().any(|j| {
            j.condition
                .as_ref()
                .is_some_and(TypedExpr::has_unbound_subquery)
        });
    exprs_pending
        || q.ctes.iter().any(|(_, c)| query_needs_binding(c))
        || q.set_operation
            .as_ref()
            .is_some_and(|s| query_needs_binding(&s.left) || query_needs_binding(&s.right))
        || q.from
            .iter()
            .chain(q.joins.iter().map(|j| &j.right_table))
            .any(|t| {
                t.subquery
                    .as_ref()
                    .is_some_and(|s| query_needs_binding(&s.query))
            })
}

/// Every expression an `AnalyzedQuery` owns directly (not those of nested
/// query scopes). `aggregates` duplicates the projection's aggregate calls
/// and is bound too, or the copy the hash aggregate evaluates would keep the
/// unbound node.
fn query_expr_slots(q: &mut AnalyzedQuery) -> Vec<&mut TypedExpr> {
    let mut slots: Vec<&mut TypedExpr> = Vec::new();
    slots.extend(q.projection.iter_mut().map(|(e, _)| e));
    slots.extend(q.selection.iter_mut());
    slots.extend(q.having.iter_mut());
    slots.extend(q.group_by.iter_mut());
    slots.extend(q.order_by.iter_mut().map(|o| &mut o.expr));
    slots.extend(q.joins.iter_mut().filter_map(|j| j.condition.as_mut()));
    for agg in &mut q.aggregates {
        slots.extend(agg.args.iter_mut());
        slots.extend(agg.filter.iter_mut());
        slots.extend(agg.order_by.iter_mut().map(|(e, _)| e));
    }
    if let Some(AnalyzedDistinct::On(exprs)) = &mut q.distinct {
        slots.extend(exprs.iter_mut());
    }
    slots
}

fn literal_for(kind: BindKind, rows: Vec<Row>) -> Result<Literal, Error> {
    match kind {
        BindKind::Exists => Ok(Literal::Boolean(!rows.is_empty())),
        BindKind::Scalar => {
            if rows.len() > 1 {
                return Err(Error::Validation(format!(
                    "scalar subquery returned {} rows; it must return at most one",
                    rows.len()
                )));
            }
            match rows.into_iter().next() {
                None => Ok(Literal::Null),
                Some(row) => match row.columns.values().next() {
                    Some(v) => from_property_value(v).map_err(Error::Validation),
                    None => Ok(Literal::Null),
                },
            }
        }
        BindKind::Array => {
            let mut items = Vec::with_capacity(rows.len());
            for row in rows {
                let lit = match row.columns.values().next() {
                    Some(v) => from_property_value(v).map_err(Error::Validation)?,
                    None => Literal::Null,
                };
                items.push(literal_to_json(&lit));
            }
            Ok(Literal::JsonB(serde_json::Value::Array(items)))
        }
    }
}

fn literal_to_json(lit: &Literal) -> serde_json::Value {
    use serde_json::Value;
    match lit {
        Literal::Null => Value::Null,
        Literal::Boolean(b) => Value::Bool(*b),
        Literal::Int(i) => Value::from(*i),
        Literal::BigInt(i) => Value::from(*i),
        Literal::Double(d) => Value::from(*d),
        Literal::Text(s) | Literal::Uuid(s) | Literal::Path(s) | Literal::Parameter(s) => {
            Value::String(s.clone())
        }
        Literal::JsonB(v) | Literal::Geometry(v) => v.clone(),
        Literal::Vector(v) => Value::Array(v.iter().map(|f| Value::from(*f)).collect()),
        Literal::Timestamp(ts) => Value::String(ts.to_rfc3339()),
        Literal::Interval(d) => Value::String(d.to_string()),
    }
}

/// Pre-order substitution mirroring the pre-order collection in
/// `bind_slots`, so the n-th result lands on the n-th subquery.
fn substitute(expr: &mut TypedExpr, results: &mut VecDeque<Literal>) {
    let replacement = match &expr.expr {
        Expr::Exists { negated, .. } => {
            let lit = results.pop_front().unwrap_or(Literal::Null);
            let value = match lit {
                Literal::Boolean(b) => Literal::Boolean(b != *negated),
                other => other,
            };
            Some(Expr::Literal(value))
        }
        Expr::ScalarSubquery { .. } => {
            Some(Expr::Literal(results.pop_front().unwrap_or(Literal::Null)))
        }
        Expr::QuantifiedSubquery { left, op, all, .. } => {
            let array = results.pop_front().unwrap_or(Literal::Null);
            Some(Expr::Quantified {
                left: left.clone(),
                op: *op,
                right: Box::new(TypedExpr::new(
                    Expr::Literal(array),
                    raisin_sql::analyzer::DataType::JsonB,
                )),
                all: *all,
            })
        }
        _ => None,
    };
    if let Some(new_expr) = replacement {
        expr.expr = new_expr;
    }
    expr.for_each_child_mut(&mut |child| substitute(child, results));
}
