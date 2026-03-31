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

            // 1. Build workspace catalog from storage
            let workspaces = deps
                .storage
                .workspaces()
                .list(RepoScope::new(&tenant, &repo))
                .await?;
            let mut catalog = StaticCatalog::default_nodes_schema();
            for ws in &workspaces {
                catalog.register_workspace(ws.name.clone());
            }

            // 2. Create QueryEngine with catalog and optional engines
            let mut engine = QueryEngine::new(deps.storage.clone(), &tenant, &repo, &branch)
                .with_catalog(Arc::new(catalog));

            if let Some(idx) = &deps.indexing_engine {
                engine = engine.with_indexing_engine(idx.clone());
            }
            if let Some(hnsw) = &deps.hnsw_engine {
                engine = engine.with_hnsw_engine(hnsw.clone());
            }
            // Use system context if no auth provided (for trigger/function execution)
            let auth = auth.unwrap_or_else(AuthContext::system);
            engine = engine.with_auth(auth);

            // 3. Substitute parameters into SQL
            let final_sql = substitute_params(&sql, &params);

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

            // 1. Build workspace catalog from storage
            let workspaces = deps
                .storage
                .workspaces()
                .list(RepoScope::new(&tenant, &repo))
                .await?;
            let mut catalog = StaticCatalog::default_nodes_schema();
            for ws in &workspaces {
                catalog.register_workspace(ws.name.clone());
            }

            // 2. Create QueryEngine with catalog and optional engines
            let mut engine = QueryEngine::new(deps.storage.clone(), &tenant, &repo, &branch)
                .with_catalog(Arc::new(catalog));

            if let Some(idx) = &deps.indexing_engine {
                engine = engine.with_indexing_engine(idx.clone());
            }
            if let Some(hnsw) = &deps.hnsw_engine {
                engine = engine.with_hnsw_engine(hnsw.clone());
            }
            // Use system context if no auth provided (for trigger/function execution)
            let auth = auth.unwrap_or_else(AuthContext::system);
            engine = engine.with_auth(auth);

            // 3. Substitute parameters into SQL
            let final_sql = substitute_params(&sql, &params);

            tracing::debug!(final_sql = %final_sql, "Executing substituted SQL");

            // 4. Execute and count affected rows
            let mut stream = engine.execute(&final_sql).await?;
            let mut row_count: i64 = 0;

            while let Some(_row_result) = stream.next().await {
                row_count += 1;
            }

            tracing::debug!(row_count = row_count, "SQL execute completed");

            Ok(row_count)
        })
    })
}

/// Substitute $1, $2, etc. with actual parameter values.
///
/// This is a simple string-based substitution. For production use,
/// consider proper prepared statement support.
fn substitute_params(sql: &str, params: &[Value]) -> String {
    let mut result = sql.to_string();
    for (i, param) in params.iter().enumerate() {
        let placeholder = format!("${}", i + 1);
        let value_str = match param {
            Value::String(s) => format!("'{}'", s.replace('\'', "''")),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "NULL".to_string(),
            Value::Array(arr) => {
                // Convert array to SQL array literal
                let items: Vec<String> = arr.iter().map(json_value_to_sql).collect();
                format!("ARRAY[{}]", items.join(", "))
            }
            Value::Object(_) => {
                // Convert object to JSON string
                format!("'{}'", param.to_string().replace('\'', "''"))
            }
        };
        result = result.replace(&placeholder, &value_str);
    }
    result
}

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
