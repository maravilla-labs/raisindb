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

//! Simple single-statement transaction write callbacks.
//!
//! Covers create, add, put, and upsert operations that each
//! generate a single SQL INSERT or UPSERT statement.

use std::sync::Arc;

use futures::StreamExt;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::Value;

use super::super::helpers::{parse_node_create_data, parse_node_full_data, substitute_params};
use super::super::store::TransactionStore;
use crate::api::{TxAddCallback, TxCreateCallback, TxPutCallback, TxUpsertCallback};
use crate::execution::callbacks::sql_generator;

/// Create the tx_create callback.
///
/// Creates a new node using SQL INSERT within the transaction.
pub fn create_tx_create<S>(store: Arc<TransactionStore<S>>) -> TxCreateCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(
        move |tx_id: String, workspace: String, parent_path: String, data: Value| {
            let store = store.clone();

            Box::pin(async move {
                let engine = store.get(&tx_id).ok_or_else(|| {
                    raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
                })?;

                let node = parse_node_create_data(&parent_path, data)?;
                let stmt = sql_generator::generate_insert(&workspace, &node);
                let final_sql = substitute_params(&stmt.sql, &stmt.params);

                let engine_guard = engine.lock().await;
                let mut stream = engine_guard.execute(&final_sql).await?;
                while stream.next().await.is_some() {}

                tracing::debug!(
                    workspace = %workspace,
                    path = %node.path,
                    "Created node via SQL INSERT in transaction"
                );

                Ok(serde_json::to_value(node).unwrap_or_default())
            })
        },
    )
}

/// Create the tx_add callback.
///
/// Adds a node with provided path using SQL INSERT.
pub fn create_tx_add<S>(store: Arc<TransactionStore<S>>) -> TxAddCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |tx_id: String, workspace: String, data: Value| {
        let store = store.clone();

        Box::pin(async move {
            let engine = store.get(&tx_id).ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
            })?;

            let node = parse_node_full_data(data)?;
            let stmt = sql_generator::generate_insert(&workspace, &node);
            let final_sql = substitute_params(&stmt.sql, &stmt.params);

            let engine_guard = engine.lock().await;
            let mut stream = engine_guard.execute(&final_sql).await?;
            while stream.next().await.is_some() {}

            Ok(serde_json::to_value(node).unwrap_or_default())
        })
    })
}

/// Create the tx_put callback.
///
/// Creates or updates a node by ID using SQL UPSERT.
pub fn create_tx_put<S>(store: Arc<TransactionStore<S>>) -> TxPutCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |tx_id: String, workspace: String, data: Value| {
        let store = store.clone();

        Box::pin(async move {
            let engine = store.get(&tx_id).ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
            })?;

            let node = parse_node_full_data(data)?;
            let stmt = sql_generator::generate_upsert(&workspace, &node);
            let final_sql = substitute_params(&stmt.sql, &stmt.params);

            let engine_guard = engine.lock().await;
            let mut stream = engine_guard.execute(&final_sql).await?;
            while stream.next().await.is_some() {}

            Ok(())
        })
    })
}

/// Create the tx_upsert callback.
///
/// Creates or updates a node by PATH using SQL UPSERT.
pub fn create_tx_upsert<S>(store: Arc<TransactionStore<S>>) -> TxUpsertCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |tx_id: String, workspace: String, data: Value| {
        let store = store.clone();

        Box::pin(async move {
            let engine = store.get(&tx_id).ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
            })?;

            let node = parse_node_full_data(data)?;
            let stmt = sql_generator::generate_upsert(&workspace, &node);
            let final_sql = substitute_params(&stmt.sql, &stmt.params);

            let engine_guard = engine.lock().await;
            let mut stream = engine_guard.execute(&final_sql).await?;
            while stream.next().await.is_some() {}

            Ok(())
        })
    })
}
