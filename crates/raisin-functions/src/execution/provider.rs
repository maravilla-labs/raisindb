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

//! Execution provider factory for creating job system callbacks.
//!
//! This module provides a factory for creating the callbacks needed by
//! the job system. It supports different execution modes:
//!
//! - **Debug**: Prints events in color, returns stub results
//! - **Production**: Full execution with real operations (future)
//!
//! # Usage
//!
//! ```ignore
//! use raisin_functions::execution::{ExecutionProvider, ExecutionMode};
//!
//! // Create debug callbacks (prints events in color)
//! let callbacks = ExecutionProvider::create_callbacks(ExecutionMode::Debug);
//!
//! // Use with init_job_system
//! let (pool, token) = storage.init_job_system(
//!     indexing_engine,
//!     hnsw_engine,
//!     callbacks.sql_executor,
//!     None,
//!     callbacks.function_executor,
//!     callbacks.function_enabled_checker,
//!     trigger_matcher,
//!     None,
//!     callbacks.binary_retrieval,
//! ).await?;
//! ```

use super::callbacks;
use super::debug::{
    create_debug_binary_retrieval, create_debug_enabled_checker, create_debug_function_executor,
    create_debug_function_executor_with_storage, create_debug_sql_executor,
};
use super::executor;
use super::types::{
    ExecutionCallbacks, ExecutionDependencies, ExecutionMode, FunctionExecutionConfig,
    SqlExecutorCallback,
};
use raisin_binary::BinaryStorage;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use std::sync::Arc;

/// Factory for creating execution callbacks.
///
/// Use this to create the callbacks needed by `init_job_system()`.
pub struct ExecutionProvider;

impl ExecutionProvider {
    /// Create execution callbacks for the specified mode.
    ///
    /// # Arguments
    ///
    /// * `mode` - The execution mode (Debug or Production)
    ///
    /// # Returns
    ///
    /// An `ExecutionCallbacks` struct containing all the callbacks.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let callbacks = ExecutionProvider::create_callbacks(ExecutionMode::Debug);
    /// ```
    pub fn create_callbacks(mode: ExecutionMode) -> ExecutionCallbacks {
        match mode {
            ExecutionMode::Debug => Self::create_debug_callbacks(),
            ExecutionMode::Production => {
                // TODO: Implement production callbacks
                // For now, panic with a clear message
                panic!(
                    "Production mode not yet implemented. \
                    Use ExecutionMode::Debug for now, or implement production callbacks \
                    by moving the code from backup_main_callbacks.rs"
                );
            }
        }
    }

    /// Create execution callbacks for the specified mode with storage access.
    ///
    /// This variant provides storage access to callbacks that need to fetch nodes.
    /// The function executor will fetch the triggering node and build a FunctionContext.
    ///
    /// # Arguments
    ///
    /// * `mode` - The execution mode (Debug or Production)
    /// * `storage` - Arc-wrapped storage for node access
    ///
    /// # Returns
    ///
    /// An `ExecutionCallbacks` struct containing all the callbacks.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let callbacks = ExecutionProvider::create_callbacks_with_storage(
    ///     ExecutionMode::Debug,
    ///     storage.clone(),
    /// );
    /// ```
    pub fn create_callbacks_with_storage<S>(
        mode: ExecutionMode,
        storage: Arc<S>,
    ) -> ExecutionCallbacks
    where
        S: Storage + 'static,
    {
        match mode {
            ExecutionMode::Debug => Self::create_debug_callbacks_with_storage(storage),
            ExecutionMode::Production => {
                // TODO: Implement production callbacks
                panic!(
                    "Production mode not yet implemented. \
                    Use ExecutionMode::Debug for now, or implement production callbacks \
                    by moving the code from backup_main_callbacks.rs"
                );
            }
        }
    }

    /// Create debug callbacks that print events in color.
    ///
    /// This is useful during development and refactoring to verify
    /// that the job system is correctly routing events.
    fn create_debug_callbacks() -> ExecutionCallbacks {
        ExecutionCallbacks {
            sql_executor: Some(create_debug_sql_executor()),
            function_executor: Some(create_debug_function_executor()),
            function_enabled_checker: Some(create_debug_enabled_checker()),
            binary_retrieval: Some(create_debug_binary_retrieval()),
        }
    }

    /// Create debug callbacks with storage access.
    ///
    /// This variant provides storage access to the function executor,
    /// allowing it to fetch nodes and build FunctionContext.
    fn create_debug_callbacks_with_storage<S>(storage: Arc<S>) -> ExecutionCallbacks
    where
        S: Storage + 'static,
    {
        ExecutionCallbacks {
            sql_executor: Some(create_debug_sql_executor()),
            function_executor: Some(create_debug_function_executor_with_storage(storage)),
            function_enabled_checker: Some(create_debug_enabled_checker()),
            binary_retrieval: Some(create_debug_binary_retrieval()),
        }
    }

    /// Create callbacks from environment configuration.
    ///
    /// Checks the `RAISIN_EXECUTION_MODE` environment variable:
    /// - `debug` or unset: Debug mode (colored output)
    /// - `production`: Production mode (real execution)
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Set environment variable: RAISIN_EXECUTION_MODE=debug
    /// let callbacks = ExecutionProvider::from_env();
    /// ```
    pub fn from_env() -> ExecutionCallbacks {
        let mode = std::env::var("RAISIN_EXECUTION_MODE")
            .map(|v| match v.to_lowercase().as_str() {
                "production" | "prod" => ExecutionMode::Production,
                _ => ExecutionMode::Debug,
            })
            .unwrap_or(ExecutionMode::Debug);

        Self::create_callbacks(mode)
    }

    /// Create production execution callbacks with full dependencies.
    ///
    /// This is the recommended way to create callbacks for production use.
    /// It bundles all dependencies into `ExecutionDependencies` and creates
    /// callbacks that perform real operations (function execution, SQL, HTTP, AI).
    ///
    /// # Arguments
    ///
    /// * `deps` - All dependencies bundled together (storage, binary storage, engines, etc.)
    /// * `config` - Configuration for function execution (timeout, memory limits, etc.)
    ///
    /// # Example
    ///
    /// ```ignore
    /// use raisin_functions::execution::{
    ///     ExecutionDependencies, ExecutionProvider, FunctionExecutionConfig
    /// };
    ///
    /// let deps = Arc::new(ExecutionDependencies {
    ///     storage: storage.clone(),
    ///     binary_storage: binary_storage.clone(),
    ///     indexing_engine: Some(indexing_engine.clone()),
    ///     hnsw_engine: Some(hnsw_engine.clone()),
    ///     http_client: reqwest::Client::new(),
    ///     ai_config_store: Some(ai_config_store.clone()),
    ///     job_registry: Some(storage.job_registry().clone()),
    ///     job_data_store: Some(storage.job_data_store().clone()),
    /// });
    ///
    /// let callbacks = ExecutionProvider::create_callbacks_with_deps(
    ///     deps,
    ///     FunctionExecutionConfig::default(),
    /// );
    ///
    /// // Use with init_job_system
    /// let (pool, token) = storage.init_job_system(
    ///     // ... other args ...
    ///     callbacks.function_executor,
    ///     callbacks.function_enabled_checker,
    ///     // ... other args ...
    ///     callbacks.binary_retrieval,
    /// ).await?;
    /// ```
    pub fn create_callbacks_with_deps<S, B>(
        deps: Arc<ExecutionDependencies<S, B>>,
        config: FunctionExecutionConfig,
    ) -> ExecutionCallbacks
    where
        S: Storage + TransactionalStorage + 'static,
        B: BinaryStorage + 'static,
    {
        ExecutionCallbacks {
            sql_executor: Some(create_sql_executor(
                deps.storage.clone(),
                deps.indexing_engine.clone(),
                deps.hnsw_engine.clone(),
            )),
            function_executor: Some(executor::create_function_executor(
                deps.clone(),
                config.clone(),
            )),
            function_enabled_checker: Some(executor::create_function_checker(
                deps.storage.clone(),
                config.functions_workspace.clone(),
            )),
            binary_retrieval: Some(callbacks::create_binary_retrieval(
                deps.binary_storage.clone(),
            )),
        }
    }
}

/// Build a real bulk-SQL executor backed by `QueryEngine`.
///
/// Async bulk `UPDATE`/`DELETE` jobs (any WHERE clause that isn't a simple
/// `id = '…'` / `path = '…'`) are dispatched to the `BulkSql` job handler, which
/// delegates to this callback. It runs the SQL synchronously through a fresh
/// `QueryEngine` built **without** a `job_registrar` — we are already inside a
/// job and must execute the bulk op inline rather than enqueue another one.
///
/// Executing the writes through the normal commit path is what emits
/// `NodeEvent`s, so node-event triggers fire for SQL-driven changes. Returning
/// the stub (which wrote nothing and reported 0 rows) silently dropped every
/// async bulk write and never fired triggers.
///
/// RLS is preserved: the engine runs under `auth` — the identity that submitted
/// the original request, captured at enqueue time — so the deferred write can
/// touch exactly the rows the caller could touch synchronously. It is **never**
/// escalated to system. If the identity is missing the engine runs anonymous
/// (fails closed) rather than over-privileged.
fn create_sql_executor<S>(
    storage: Arc<S>,
    indexing_engine: Option<Arc<raisin_indexer::TantivyIndexingEngine>>,
    hnsw_engine: Option<Arc<raisin_hnsw::HnswIndexingEngine>>,
) -> SqlExecutorCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    use futures::StreamExt;
    use raisin_models::auth::AuthContext;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_sql_execution::{QueryEngine, StaticCatalog};
    use raisin_storage::{RepoScope, WorkspaceRepository};

    Arc::new(
        move |sql: String,
              tenant_id: String,
              repo_id: String,
              branch: String,
              actor: String,
              auth: Option<AuthContext>| {
            let storage = storage.clone();
            let indexing_engine = indexing_engine.clone();
            let hnsw_engine = hnsw_engine.clone();

            Box::pin(async move {
                // Register every workspace so `FROM <workspace>` resolves.
                let workspaces = storage
                    .workspaces()
                    .list(RepoScope::new(&tenant_id, &repo_id))
                    .await?;
                let mut catalog = StaticCatalog::default_nodes_schema();
                for ws in &workspaces {
                    catalog.register_workspace(ws.name.clone());
                }

                // Run under the submitter's identity (captured at enqueue time)
                // so RLS matches a synchronous request. Fall back to anonymous —
                // never system — when the identity is absent.
                let auth = auth.unwrap_or_else(AuthContext::anonymous);
                let mut engine =
                    QueryEngine::new(storage.clone(), &tenant_id, &repo_id, &branch)
                        .with_catalog(Arc::new(catalog))
                        .with_auth(auth)
                        .with_default_actor(actor);
                if let Some(idx) = &indexing_engine {
                    engine = engine.with_indexing_engine(idx.clone());
                }
                if let Some(hnsw) = &hnsw_engine {
                    engine = engine.with_hnsw_engine(hnsw.clone());
                }

                // Forced-sync execution: do not enqueue another async bulk job.
                let mut stream = engine.execute_batch_sync(&sql).await?;
                let mut affected_rows = 0i64;
                while let Some(row) = stream.next().await {
                    let row = row?;
                    match row.columns.get("affected_rows") {
                        Some(PropertyValue::Integer(n)) => affected_rows += *n,
                        Some(PropertyValue::Float(f)) => affected_rows += *f as i64,
                        _ => {}
                    }
                }
                Ok(affected_rows)
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_debug_callbacks() {
        let callbacks = ExecutionProvider::create_callbacks(ExecutionMode::Debug);
        assert!(callbacks.sql_executor.is_some());
        assert!(callbacks.function_executor.is_some());
        assert!(callbacks.function_enabled_checker.is_some());
        assert!(callbacks.binary_retrieval.is_some());
    }

    #[test]
    #[should_panic(expected = "Production mode not yet implemented")]
    fn test_create_production_callbacks_panics() {
        let _ = ExecutionProvider::create_callbacks(ExecutionMode::Production);
    }

    #[test]
    fn test_from_env_defaults_to_debug() {
        // Clear the env var to ensure default behavior
        std::env::remove_var("RAISIN_EXECUTION_MODE");
        let callbacks = ExecutionProvider::from_env();
        assert!(callbacks.sql_executor.is_some());
    }
}
