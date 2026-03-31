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

//! Transaction callbacks for delete and move operations.
//!
//! Covers delete (by path), delete_by_id, and move operations
//! that remove or relocate nodes within a transaction.

use std::sync::Arc;

use futures::StreamExt;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::Value;

use super::super::helpers::{row_to_json_object, substitute_params};
use super::super::store::TransactionStore;
use crate::api::{TxDeleteByIdCallback, TxDeleteCallback, TxMoveCallback};
use crate::execution::callbacks::sql_generator;

/// Create the tx_delete callback.
///
/// Deletes a node by path using SQL DELETE.
pub fn create_tx_delete<S>(store: Arc<TransactionStore<S>>) -> TxDeleteCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |tx_id: String, workspace: String, path: String| {
        let store = store.clone();

        Box::pin(async move {
            let engine = store.get(&tx_id).ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
            })?;

            let stmt = sql_generator::generate_delete_cascade(&workspace, &path);
            let final_sql = substitute_params(&stmt.sql, &stmt.params);

            let engine_guard = engine.lock().await;
            let mut stream = engine_guard.execute(&final_sql).await?;
            while stream.next().await.is_some() {}

            Ok(())
        })
    })
}

/// Create the tx_delete_by_id callback.
///
/// Deletes a node by ID using SQL DELETE.
pub fn create_tx_delete_by_id<S>(store: Arc<TransactionStore<S>>) -> TxDeleteByIdCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |tx_id: String, workspace: String, id: String| {
        let store = store.clone();

        Box::pin(async move {
            let engine = store.get(&tx_id).ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
            })?;

            let sql = format!(
                "DELETE FROM {} WHERE id = '{}'",
                workspace,
                id.replace('\'', "''")
            );

            let engine_guard = engine.lock().await;
            let mut stream = engine_guard.execute(&sql).await?;
            while stream.next().await.is_some() {}

            Ok(())
        })
    })
}

/// Create the tx_move callback.
///
/// Moves a node to a new parent using SQL MOVE statement.
pub fn create_tx_move<S>(store: Arc<TransactionStore<S>>) -> TxMoveCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(
        move |tx_id: String, workspace: String, node_path: String, new_parent_path: String| {
            let store = store.clone();

            Box::pin(async move {
                let engine = store.get(&tx_id).ok_or_else(|| {
                    raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
                })?;

                // Extract node name from current path
                let node_name = node_path.split('/').next_back().ok_or_else(|| {
                    raisin_error::Error::Validation("Invalid node path".to_string())
                })?;

                // Build new path
                let new_path = if new_parent_path == "/" {
                    format!("/{}", node_name)
                } else {
                    format!("{}/{}", new_parent_path, node_name)
                };

                tracing::debug!(
                    old_path = %node_path,
                    new_path = %new_path,
                    "Moving node via SQL MOVE in transaction"
                );

                let stmt = sql_generator::generate_move(&workspace, &node_path, &new_path);
                let final_sql = substitute_params(&stmt.sql, &stmt.params);

                let engine_guard = engine.lock().await;
                let mut stream = engine_guard.execute(&final_sql).await?;
                while stream.next().await.is_some() {}

                // Fetch and return the moved node
                let get_stmt = sql_generator::generate_select_by_path(&workspace, &new_path);
                let get_sql = substitute_params(&get_stmt.sql, &get_stmt.params);
                let mut get_stream = engine_guard.execute(&get_sql).await?;

                let result = if let Some(row_result) = get_stream.next().await {
                    let row = row_result?;
                    let obj = row_to_json_object(row);
                    Some(Value::Object(obj))
                } else {
                    None
                };

                result.ok_or_else(|| {
                    raisin_error::Error::Internal(format!(
                        "Failed to retrieve moved node at: {}",
                        new_path
                    ))
                })
            })
        },
    )
}
