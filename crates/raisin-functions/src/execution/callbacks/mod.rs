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

//! Production callback implementations for RaisinFunctionApi.
//!
//! This module provides factory functions for creating all the callbacks
//! needed by `RaisinFunctionApiCallbacks`. Each callback type is organized
//! into its own submodule for maintainability.

pub mod ai;
pub mod events;
pub mod functions;
pub mod http;
pub mod nodes;
pub mod query_context;
pub mod resources;
pub mod sql;
pub mod sql_generator;
pub mod tasks;
pub mod transaction;

use std::sync::Arc;

use raisin_binary::BinaryStorage;
use raisin_models::auth::AuthContext;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;

use super::types::ExecutionDependencies;
use crate::api::RaisinFunctionApiCallbacks;

/// Create all production callbacks from dependencies.
///
/// This is the main factory function that assembles all API callbacks
/// for function execution. Each callback is created with access to the
/// relevant dependencies (storage, SQL engine, HTTP client, AI config).
///
/// All node operations now route through SQL to ensure consistent transaction
/// handling and auto-commit behavior. This makes ESQL the single source of truth
/// for all data operations.
///
/// The `auth_context` parameter controls RLS filtering for node operations.
/// - `None`: Operations run without auth (system context, no RLS filtering)
/// - `Some(auth)`: Operations are filtered based on user's permissions
pub fn create_production_callbacks<S, B>(
    deps: Arc<ExecutionDependencies<S, B>>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> RaisinFunctionApiCallbacks
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    // Create shared QueryContext for SQL-based operations
    let query_ctx = Arc::new(query_context::QueryContext::new(
        deps.clone(),
        tenant_id.clone(),
        repo_id.clone(),
        branch.clone(),
        auth_context.clone(),
    ));

    // Create transaction store for this execution context (typed with storage)
    let tx_store = Arc::new(transaction::TransactionStore::new());

    RaisinFunctionApiCallbacks {
        // Node operations - all routed through SQL for consistent auto-commit
        node_get: Some(nodes::create_node_get(query_ctx.clone())),
        node_get_by_id: Some(nodes::create_node_get_by_id(query_ctx.clone())),
        node_get_children: Some(nodes::create_node_get_children(query_ctx.clone())),
        node_query: Some(nodes::create_node_query(query_ctx.clone())),
        node_create: Some(nodes::create_node_create(query_ctx.clone())),
        node_update: Some(nodes::create_node_update(query_ctx.clone())),
        node_delete: Some(nodes::create_node_delete(query_ctx.clone())),
        node_update_property: Some(nodes::create_node_update_property(query_ctx.clone())),
        node_move: Some(nodes::create_node_move(query_ctx.clone())),
        node_add_resource: Some(resources::create_node_add_resource(
            deps.storage.clone(),
            deps.binary_storage.clone(),
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            auth_context.clone(),
        )),

        // SQL operations
        sql_query: Some(sql::create_sql_query(
            deps.clone(),
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            auth_context.clone(),
        )),
        sql_execute: Some(sql::create_sql_execute(
            deps.clone(),
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            auth_context.clone(),
        )),

        // HTTP operations
        http_request: Some(http::create_http_request(deps.http_client.clone())),

        // Event operations
        emit_event: Some(events::create_emit_event()),

        // AI operations
        ai_completion: Some(ai::create_ai_completion(
            deps.ai_config_store.clone(),
            tenant_id.clone(),
        )),
        ai_embed: Some(ai::create_ai_embed(
            deps.ai_config_store.clone(),
            tenant_id.clone(),
        )),
        ai_list_models: Some(ai::create_ai_list_models(
            deps.ai_config_store.clone(),
            tenant_id.clone(),
        )),
        ai_get_default_model: Some(ai::create_ai_get_default_model(
            deps.ai_config_store.clone(),
            tenant_id.clone(),
        )),
        // Resource operations
        resource_get_binary: Some(resources::create_resource_get_binary(
            deps.binary_storage.clone(),
        )),

        // Function execution - only available if job system dependencies are provided
        function_execute: match (&deps.job_registry, &deps.job_data_store) {
            (Some(registry), Some(data_store)) => Some(functions::create_function_execute(
                deps.clone(),
                registry.clone(),
                data_store.clone(),
                tenant_id.clone(),
                repo_id.clone(),
                branch.clone(),
            )),
            _ => None,
        },

        // Function call (simple function-to-function calls) - only available if job system dependencies are provided
        function_call: match (&deps.job_registry, &deps.job_data_store) {
            (Some(registry), Some(data_store)) => Some(functions::create_function_call(
                registry.clone(),
                data_store.clone(),
                tenant_id.clone(),
                repo_id.clone(),
                branch.clone(),
            )),
            _ => None,
        },

        // Task operations
        task_create: Some(tasks::create_task_create(
            deps.storage.clone(),
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            auth_context.clone(),
        )),
        task_update: Some(tasks::create_task_update(
            deps.storage.clone(),
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            auth_context.clone(),
        )),
        task_complete: Some(tasks::create_task_complete(
            deps.storage.clone(),
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            auth_context.clone(),
        )),
        task_query: Some(tasks::create_task_query(
            deps.storage.clone(),
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            auth_context.clone(),
        )),

        // Transaction callbacks - now use SQL BEGIN/COMMIT for consistent behavior
        tx_begin: Some(transaction::create_tx_begin(
            query_ctx.clone(),
            tx_store.clone(),
        )),
        tx_commit: Some(transaction::create_tx_commit(tx_store.clone())),
        tx_rollback: Some(transaction::create_tx_rollback(tx_store.clone())),
        tx_set_actor: Some(transaction::create_tx_set_actor(tx_store.clone())),
        tx_set_message: Some(transaction::create_tx_set_message(tx_store.clone())),
        tx_create: Some(transaction::create_tx_create(tx_store.clone())),
        tx_add: Some(transaction::create_tx_add(tx_store.clone())),
        tx_put: Some(transaction::create_tx_put(tx_store.clone())),
        tx_upsert: Some(transaction::create_tx_upsert(tx_store.clone())),
        tx_create_deep: Some(transaction::create_tx_create_deep(tx_store.clone())),
        tx_upsert_deep: Some(transaction::create_tx_upsert_deep(tx_store.clone())),
        tx_update: Some(transaction::create_tx_update(tx_store.clone())),
        tx_delete: Some(transaction::create_tx_delete(tx_store.clone())),
        tx_delete_by_id: Some(transaction::create_tx_delete_by_id(tx_store.clone())),
        tx_get: Some(transaction::create_tx_get(tx_store.clone())),
        tx_get_by_path: Some(transaction::create_tx_get_by_path(tx_store.clone())),
        tx_list_children: Some(transaction::create_tx_list_children(tx_store.clone())),
        tx_move: Some(transaction::create_tx_move(tx_store.clone())),
        tx_update_property: Some(transaction::create_tx_update_property(tx_store.clone())),

        // PDF processing - storage-key based processing (no base64 overhead)
        pdf_process_from_storage: Some(resources::create_pdf_process_from_storage(
            deps.binary_storage.clone(),
        )),
    }
}

/// Create the binary retrieval callback for the job system.
///
/// This callback is used by the package install handler to retrieve
/// binary blobs from storage.
pub fn create_binary_retrieval<B>(binary_storage: Arc<B>) -> raisin_rocksdb::BinaryRetrievalCallback
where
    B: BinaryStorage + 'static,
{
    Arc::new(move |key: String| {
        let bin = binary_storage.clone();
        Box::pin(async move {
            bin.get(&key)
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(|e| {
                    raisin_error::Error::storage(format!("Failed to retrieve binary: {}", e))
                })
        })
    })
}
