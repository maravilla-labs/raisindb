// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! SQL operation callbacks for function execution.
//!
//! These callbacks implement the `raisin.sql.*` API available to JavaScript functions.

use std::sync::Arc;

use futures::StreamExt;
use raisin_binary::BinaryStorage;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::scope::RepoScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{Storage, WorkspaceRepository};
use serde_json::Value;

use super::super::types::ExecutionDependencies;
use crate::api::{SqlExecuteCallback, SqlQueryCallback};

/// Build the `QueryEngine` backing `raisin.sql.query()` / `raisin.sql.execute()`.
///
/// Both callbacks had a verbatim copy of this, and both ran it *inside* the
/// per-call closure — so the workspace listing and catalog construction happened
/// once per SQL statement a function issued, not once per function.
///
/// The catalog now comes from the shared per-repo cache
/// (`raisin_sql_execution::workspace_catalog`), the same one the HTTP, WS and
/// pgwire paths use, and the schema-stats cache is wired in — its absence here
/// meant the planner re-listed every NodeType and Archetype on every query while
/// the other transports were served from cache.
async fn build_engine<S, B>(
    deps: &ExecutionDependencies<S, B>,
    tenant: &str,
    repo: &str,
    branch: &str,
    auth: Option<AuthContext>,
) -> Result<QueryEngine<S>, raisin_error::Error>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let catalog = raisin_sql_execution::workspace_catalog(deps.storage.as_ref(), tenant, repo)
        .await
        .map_err(|e| {
            raisin_error::Error::Backend(format!("Failed to build workspace catalog: {e}"))
        })?;

    let mut engine =
        QueryEngine::new(deps.storage.clone(), tenant, repo, branch).with_catalog(catalog);

    if let Some(idx) = &deps.indexing_engine {
        engine = engine.with_indexing_engine(idx.clone());
    }
    if let Some(hnsw) = &deps.hnsw_engine {
        engine = engine.with_hnsw_engine(hnsw.clone());
    }
    if let Some(stats) = &deps.schema_stats_cache {
        engine = engine.with_schema_stats_cache(stats.clone());
    }

    // Use system context if no auth provided (for trigger/function execution)
    Ok(engine.with_auth(auth.unwrap_or_else(AuthContext::system)))
}

/// Create sql_query callback: `raisin.sql.query(sql, params)`
///
/// Executes a SQL SELECT query and returns the results as a JSON array.
/// Supports parameterized queries with $1, $2, etc. placeholders.
///
/// The `auth_context` parameter is used for RLS filtering when provided.
pub fn create_sql_query<S, B>(
    deps: Arc<ExecutionDependencies<S, B>>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> SqlQueryCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |sql: String, params: Vec<Value>| {
        let deps = deps.clone();
        let tenant = tenant_id.clone();
        let repo = repo_id.clone();
        let branch = branch.clone();
        let auth = auth_context.clone();

        Box::pin(async move {
            tracing::debug!(
                sql = %sql,
                params = ?params,
                "Executing SQL query in function context"
            );

            // 1-2. Build the engine. The catalog is shared and cached per repo:
            // this closure runs on EVERY `raisin.sql.query()` call, so a function
            // issuing 20 queries used to perform 20 full workspace listings.
            let engine = build_engine(&deps, &tenant, &repo, &branch, auth).await?;

            // 3. Substitute parameters into SQL
            let final_sql = substitute_params(&sql, &params)?;

            tracing::debug!(final_sql = %final_sql, "Executing substituted SQL");

            // 4. Execute and collect rows
            let mut stream = engine.execute(&final_sql).await?;
            let mut rows = Vec::new();

            while let Some(row_result) = stream.next().await {
                let row = row_result?;
                // Convert Row to JSON object
                let mut obj = serde_json::Map::new();
                for (key, value) in row.columns {
                    obj.insert(key, property_value_to_json(value));
                }
                rows.push(Value::Object(obj));
            }

            tracing::debug!(row_count = rows.len(), "SQL query completed");

            Ok(Value::Array(rows))
        })
    })
}

/// Create sql_execute callback: `raisin.sql.execute(sql, params)`
///
/// For DML statements (INSERT, UPDATE, DELETE, RELATE, UNRELATE, MOVE) - returns affected row count.
///
/// The `auth_context` parameter is used for permission checks.
pub fn create_sql_execute<S, B>(
    deps: Arc<ExecutionDependencies<S, B>>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> SqlExecuteCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |sql: String, params: Vec<Value>| {
        let deps = deps.clone();
        let tenant = tenant_id.clone();
        let repo = repo_id.clone();
        let branch = branch.clone();
        let auth = auth_context.clone();

        Box::pin(async move {
            tracing::debug!(
                sql = %sql,
                params = ?params,
                "Executing SQL statement in function context"
            );

            // 1-2. Build the engine (shared, cached catalog — see `build_engine`).
            let engine = build_engine(&deps, &tenant, &repo, &branch, auth).await?;

            // 3. Substitute parameters into SQL
            let final_sql = substitute_params(&sql, &params)?;

            tracing::debug!(final_sql = %final_sql, "Executing substituted SQL");

            // 4. Execute and report affected rows.
            //
            // DML (UPDATE/DELETE/INSERT) emits a single summary row carrying the
            // real `affected_rows` count. We MUST read that value — counting the
            // emitted rows would always return 1 for any DML, which silently
            // breaks callers that branch on the affected count (a guarded UPDATE
            // that matched 0 rows would look like it affected 1).
            // Fall back to counting rows for statements that don't surface the
            // summary column.
            let mut stream = engine.execute(&final_sql).await?;
            let mut affected: i64 = 0;
            let mut row_count: i64 = 0;
            let mut saw_affected = false;

            while let Some(row_result) = stream.next().await {
                let row = row_result?;
                row_count += 1;
                if let Some(value) = row.columns.get("affected_rows") {
                    match value {
                        PropertyValue::Integer(n) => {
                            affected += *n;
                            saw_affected = true;
                        }
                        PropertyValue::Float(f) => {
                            affected += *f as i64;
                            saw_affected = true;
                        }
                        _ => {}
                    }
                }
            }

            let result = if saw_affected { affected } else { row_count };
            tracing::debug!(
                affected_rows = result,
                saw_affected = saw_affected,
                "SQL execute completed"
            );

            Ok(result)
        })
    })
}

/// Substitute $1, $2, etc. with actual parameter values.
///
/// This is a simple string-based substitution. For production use,
/// consider proper prepared statement support.
use crate::execution::callbacks::sql_params::substitute_params;

/// Convert a JSON value to SQL literal string
fn json_value_to_sql(val: &Value) -> String {
    match val {
        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "NULL".to_string(),
        _ => format!("'{}'", val.to_string().replace('\'', "''")),
    }
}

/// Convert PropertyValue to JSON Value
fn property_value_to_json(pv: PropertyValue) -> Value {
    match pv {
        PropertyValue::Null => Value::Null,
        PropertyValue::Boolean(b) => Value::Bool(b),
        PropertyValue::Integer(i) => Value::Number(i.into()),
        PropertyValue::Float(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        PropertyValue::Date(d) => Value::String(d.to_rfc3339()),
        PropertyValue::Decimal(d) => Value::String(d.to_string()),
        PropertyValue::String(s) => Value::String(s),
        PropertyValue::Reference(r) => serde_json::to_value(r).unwrap_or(Value::Null),
        PropertyValue::Url(u) => serde_json::to_value(u).unwrap_or(Value::Null),
        PropertyValue::Resource(r) => serde_json::to_value(r).unwrap_or(Value::Null),
        PropertyValue::Composite(c) => serde_json::to_value(c).unwrap_or(Value::Null),
        PropertyValue::Element(e) => serde_json::to_value(e).unwrap_or(Value::Null),
        PropertyValue::Vector(v) => serde_json::to_value(v).unwrap_or(Value::Null),
        PropertyValue::Geometry(g) => serde_json::to_value(g).unwrap_or(Value::Null),
        PropertyValue::Array(arr) => {
            Value::Array(arr.into_iter().map(property_value_to_json).collect())
        }
        PropertyValue::Object(map) => {
            let obj: serde_json::Map<String, Value> = map
                .into_iter()
                .map(|(k, v)| (k, property_value_to_json(v)))
                .collect();
            Value::Object(obj)
        }
    }
}
