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

//! Read-only transaction operation callbacks.
//!
//! Provides get, get_by_path, and list_children operations that query
//! data within an active SQL transaction (seeing uncommitted changes).

use std::sync::Arc;

use futures::StreamExt;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::Value;

use super::helpers::{row_to_json_object, substitute_params};
use super::store::TransactionStore;
use crate::api::{TxGetByPathCallback, TxGetCallback, TxListChildrenCallback};
use crate::execution::callbacks::sql_generator;

/// Create the tx_get callback.
///
/// Gets a node by ID using SQL SELECT (sees uncommitted changes within transaction).
pub fn create_tx_get<S>(store: Arc<TransactionStore<S>>) -> TxGetCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |tx_id: String, workspace: String, id: String| {
        let store = store.clone();

        Box::pin(async move {
            let engine = store.get(&tx_id).ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
            })?;

            let stmt = sql_generator::generate_select_by_id(&workspace, &id);
            let final_sql = substitute_params(&stmt.sql, &stmt.params);

            let engine_guard = engine.lock().await;
            let mut stream = engine_guard.execute(&final_sql).await?;

            let result = if let Some(row_result) = stream.next().await {
                let row = row_result?;
                let obj = row_to_json_object(row);
                Some(Value::Object(obj))
            } else {
                None
            };

            Ok(result)
        })
    })
}

/// Create the tx_get_by_path callback.
///
/// Gets a node by path using SQL SELECT (sees uncommitted changes within transaction).
pub fn create_tx_get_by_path<S>(store: Arc<TransactionStore<S>>) -> TxGetByPathCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |tx_id: String, workspace: String, path: String| {
        let store = store.clone();

        Box::pin(async move {
            let engine = store.get(&tx_id).ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
            })?;

            let stmt = sql_generator::generate_select_by_path(&workspace, &path);
            let final_sql = substitute_params(&stmt.sql, &stmt.params);

            let engine_guard = engine.lock().await;
            let mut stream = engine_guard.execute(&final_sql).await?;

            let result = if let Some(row_result) = stream.next().await {
                let row = row_result?;
                let obj = row_to_json_object(row);
                Some(Value::Object(obj))
            } else {
                None
            };

            Ok(result)
        })
    })
}

/// Create the tx_list_children callback.
///
/// Lists children of a node using SQL SELECT.
pub fn create_tx_list_children<S>(store: Arc<TransactionStore<S>>) -> TxListChildrenCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(
        move |tx_id: String, workspace: String, parent_path: String| {
            let store = store.clone();

            Box::pin(async move {
                let engine = store.get(&tx_id).ok_or_else(|| {
                    raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
                })?;

                let stmt = sql_generator::generate_select_children(&workspace, &parent_path, None);
                let final_sql = substitute_params(&stmt.sql, &stmt.params);

                let engine_guard = engine.lock().await;
                let mut stream = engine_guard.execute(&final_sql).await?;

                let mut results: Vec<Value> = Vec::new();
                while let Some(row_result) = stream.next().await {
                    let row = row_result?;
                    let obj = row_to_json_object(row);
                    results.push(Value::Object(obj));
                }

                Ok(results)
            })
        },
    )
}
