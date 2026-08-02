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

//! Shared QueryEngine context for node operations.
//!
//! Provides a unified context that can create QueryEngine instances for executing
//! SQL operations, ensuring all node operations go through the SQL transaction system.

use std::sync::Arc;

use futures::StreamExt;
use raisin_binary::BinaryStorage;
use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::scope::RepoScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{Storage, WorkspaceRepository};
use serde_json::Value;

use super::super::types::ExecutionDependencies;
use super::sql_generator::SqlStatement;

/// Shared context for creating and executing SQL via QueryEngine.
///
/// This context holds all dependencies needed to create a QueryEngine and execute SQL.
/// It is shared across node callbacks to ensure consistent transaction handling.
pub struct QueryContext<S, B>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    /// Shared execution dependencies
    pub deps: Arc<ExecutionDependencies<S, B>>,
    /// Tenant ID for this context
    pub tenant_id: String,
    /// Repository ID for this context
    pub repo_id: String,
    /// Branch name for this context
    pub branch: String,
    /// Optional auth context for RLS
    pub auth_context: Option<AuthContext>,
}

impl<S, B> QueryContext<S, B>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    /// Create a new QueryContext
    pub fn new(
        deps: Arc<ExecutionDependencies<S, B>>,
        tenant_id: String,
        repo_id: String,
        branch: String,
        auth_context: Option<AuthContext>,
    ) -> Self {
        Self {
            deps,
            tenant_id,
            repo_id,
            branch,
            auth_context,
        }
    }

    /// Create a fresh QueryEngine for auto-commit operations.
    ///
    /// This creates a new QueryEngine with the workspace catalog and optional
    /// indexing engines. Each call creates a fresh engine, which means each
    /// operation will use auto-commit mode.
    pub async fn create_engine(&self) -> Result<QueryEngine<S>, Error> {
        // Build workspace catalog from storage
        let workspaces = self
            .deps
            .storage
            .workspaces()
            .list(RepoScope::new(&self.tenant_id, &self.repo_id))
            .await?;
        let mut catalog = StaticCatalog::default_nodes_schema();
        for ws in &workspaces {
            catalog.register_workspace(ws.name.clone());
        }

        // Create QueryEngine with catalog and optional engines
        let mut engine = QueryEngine::new(
            self.deps.storage.clone(),
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
        )
        .with_catalog(Arc::new(catalog));

        if let Some(idx) = &self.deps.indexing_engine {
            engine = engine.with_indexing_engine(idx.clone());
        }
        if let Some(hnsw) = &self.deps.hnsw_engine {
            engine = engine.with_hnsw_engine(hnsw.clone());
        }
        if let Some(mgr) = &self.deps.lock_manager {
            engine = engine.with_lock_manager(mgr.clone());
        }

        // Use system context if no auth provided (for trigger/function execution)
        let auth = self
            .auth_context
            .clone()
            .unwrap_or_else(AuthContext::system);
        engine = engine.with_auth(auth);

        Ok(engine)
    }

    /// Execute a SQL statement and return the result rows as JSON.
    ///
    /// This creates a new QueryEngine (auto-commit mode) and executes the SQL.
    /// The engine will automatically commit the transaction after execution.
    pub async fn execute_query(&self, stmt: &SqlStatement) -> Result<Vec<Value>, Error> {
        let engine = self.create_engine().await?;
        let final_sql = substitute_params(&stmt.sql, &stmt.params)?;

        tracing::debug!(sql = %final_sql, "Executing SQL query via QueryContext");

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

        tracing::debug!(
            row_count = rows.len(),
            "SQL query completed via QueryContext"
        );
        Ok(rows)
    }

    /// Execute a SQL statement and return the affected row count.
    ///
    /// This creates a new QueryEngine (auto-commit mode) and executes the SQL.
    /// Suitable for INSERT, UPDATE, DELETE operations.
    pub async fn execute_statement(&self, stmt: &SqlStatement) -> Result<i64, Error> {
        let engine = self.create_engine().await?;
        let final_sql = substitute_params(&stmt.sql, &stmt.params)?;

        tracing::debug!(sql = %final_sql, "Executing SQL statement via QueryContext");

        // DML emits a single summary row carrying the real `affected_rows` count;
        // read that value rather than counting emitted rows (which is always 1 for
        // DML and would break compare-and-swap callers). Fall back to a row count
        // for statements that don't surface the summary column.
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
            "SQL statement completed via QueryContext"
        );
        Ok(result)
    }

    /// Execute raw SQL string with parameters and return result rows.
    ///
    /// This is a convenience method that combines SQL string and parameters.
    pub async fn execute_sql(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Value>, Error> {
        let stmt = SqlStatement {
            sql: sql.to_string(),
            params,
        };
        self.execute_query(&stmt).await
    }
}

use crate::execution::callbacks::sql_params::substitute_params;

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

// Substitution tests live with the substitution, in `super::sql_params`.
