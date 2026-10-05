//! SELECT without FROM: one row, evaluated directly, no plan.
//!
//! This path does not go through the planner, so every clause of the query is
//! either honoured HERE or rejected HERE. A clause it silently ignores is a
//! wrong answer with no error: `SELECT 1 WHERE false` returning a row, or
//! `SELECT 'all' UNION ALL SELECT name FROM 'ws'` dropping its right side
//! (the analyzer copies the LEFT side's empty FROM onto a set operation, so a
//! set operation whose first query has no FROM arrives here too).
//!
//! Over a single row, WHERE, LIMIT and OFFSET decide whether the row exists;
//! ORDER BY and DISTINCT cannot change it. GROUP BY, HAVING and set
//! operations are rejected.

use super::super::QueryEngine;
use crate::physical_plan::eval::{eval_expr, eval_expr_async, has_async_call};
use crate::physical_plan::executor::{ExecutionContext, Row, RowStream};
use futures::stream;
use indexmap::IndexMap;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{AnalyzedQuery, AnalyzedStatement, Literal, TypedExpr};
use raisin_storage::Storage;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Run `analyzed` here when it is a SELECT without FROM; `None` sends it
    /// to the planner. The ONE gate, shared by `execute` and `execute_batch`.
    pub(crate) async fn execute_scalar_if_no_from(
        &self,
        analyzed: &AnalyzedStatement,
    ) -> Option<Result<RowStream, Error>> {
        let AnalyzedStatement::Query(q) = analyzed else {
            return None;
        };
        if !q.from.is_empty() {
            return None;
        }
        Some(self.execute_scalar_select(q).await)
    }

    async fn execute_scalar_select(&self, q: &AnalyzedQuery) -> Result<RowStream, Error> {
        reject_unsupported_clauses(q)?;
        // Decided before anything is evaluated, so a side-effecting projection
        // (INVOKE, RAISIN_TRY_ACQUIRE) never runs for a row nobody receives.
        if q.limit == Some(0) || q.offset.unwrap_or(0) > 0 {
            return Ok(no_rows());
        }
        let ctx = if query_has_async_calls(q) {
            Some(self.scalar_eval_context(q).await)
        } else {
            None
        };
        let empty_row = Row::new();
        if let Some(predicate) = &q.selection {
            if !is_true(eval(predicate, &empty_row, ctx.as_ref()).await?)? {
                return Ok(no_rows());
            }
        }
        let mut columns = IndexMap::new();
        for (i, (expr, alias)) in q.projection.iter().enumerate() {
            let value = eval(expr, &empty_row, ctx.as_ref()).await?;
            let name = alias.clone().unwrap_or_else(|| format!("column{}", i + 1));
            columns.insert(name, literal_to_property_value(value));
        }
        let row = Row::from_map(columns);
        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }

    /// The context the async evaluator needs (RESOLVE_PATH, RESOLVE,
    /// EMBEDDING, INVOKE, the lock functions …): THE context builder every
    /// planned query runs in, so a FROM-less call sees the same storage, auth,
    /// embedding, invoke and lock wiring as one with a FROM clause.
    async fn scalar_eval_context(&self, q: &AnalyzedQuery) -> ExecutionContext<S> {
        let branch = match &q.branch_override {
            Some(branch) => branch.clone(),
            None => self.effective_branch().await,
        };
        self.build_execution_context(
            branch,
            "default".to_string(),
            q.max_revision,
            q.locales.clone(),
        )
    }
}

/// Clauses that need more than one row to mean anything.
fn reject_unsupported_clauses(q: &AnalyzedQuery) -> Result<(), Error> {
    let clause = if let Some(set_op) = &q.set_operation {
        set_op.kind.keyword()
    } else if !q.group_by.is_empty() {
        "GROUP BY"
    } else if q.having.is_some() {
        "HAVING"
    } else {
        return Ok(());
    };
    Err(Error::Validation(format!(
        "{clause} is not supported on a SELECT without a FROM clause"
    )))
}

async fn eval<S: Storage>(
    expr: &TypedExpr,
    row: &Row,
    ctx: Option<&ExecutionContext<S>>,
) -> Result<Literal, Error> {
    match ctx {
        Some(ctx) => eval_expr_async(expr, row, ctx).await,
        None => eval_expr(expr, row),
    }
}

/// WHERE semantics of the Filter operator: NULL and false drop the row.
fn is_true(value: Literal) -> Result<bool, Error> {
    match value {
        Literal::Boolean(b) => Ok(b),
        Literal::Null => Ok(false),
        other => Err(Error::Validation(format!(
            "Filter predicate must return boolean, got {other:?}"
        ))),
    }
}

fn no_rows() -> RowStream {
    Box::pin(stream::empty())
}

/// Whether a scalar SELECT's projection or WHERE clause calls a function only
/// the async evaluator can run — asked of the ONE registry of such functions
/// (`eval::has_async_call`). A second list here once knew INVOKE and the lock
/// functions but not RESOLVE_PATH, so `SELECT RESOLVE_PATH(…) AS id` with no
/// FROM failed with "Unknown function" while the same call with a FROM worked.
fn query_has_async_calls(q: &AnalyzedQuery) -> bool {
    q.projection.iter().any(|(expr, _)| has_async_call(expr))
        || q.selection.as_ref().is_some_and(has_async_call)
}

/// Convert a Literal to PropertyValue (shared between scalar and async scalar paths).
fn literal_to_property_value(value: Literal) -> PropertyValue {
    match value {
        Literal::Null => PropertyValue::Null,
        Literal::Boolean(b) => PropertyValue::Boolean(b),
        Literal::Int(n) => PropertyValue::Integer(n as i64),
        Literal::BigInt(n) => PropertyValue::Integer(n),
        Literal::Double(f) => PropertyValue::Float(f),
        Literal::Text(s) | Literal::Uuid(s) | Literal::Path(s) => PropertyValue::String(s),
        Literal::JsonB(json) => match json {
            serde_json::Value::Null => PropertyValue::Null,
            serde_json::Value::Bool(b) => PropertyValue::Boolean(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    PropertyValue::Integer(i)
                } else if let Some(f) = n.as_f64() {
                    PropertyValue::Float(f)
                } else {
                    PropertyValue::String(n.to_string())
                }
            }
            serde_json::Value::String(s) => PropertyValue::String(s),
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                PropertyValue::String(json.to_string())
            }
        },
        Literal::Timestamp(t) => PropertyValue::Date(t.into()),
        Literal::Vector(v) => PropertyValue::Vector(v),
        Literal::Geometry(geojson) => match serde_json::from_value(geojson) {
            Ok(geo) => PropertyValue::Geometry(geo),
            Err(_) => PropertyValue::Null,
        },
        Literal::Interval(_) | Literal::Parameter(_) => PropertyValue::Null,
    }
}
