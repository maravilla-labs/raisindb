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

//! Transaction callbacks for updating existing node data.
//!
//! Covers update (full property merge) and update_property (single property)
//! operations that modify existing nodes within a transaction.

use std::sync::Arc;

use futures::StreamExt;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::Value;

use super::super::helpers::{
    apply_node_updates, json_to_property_value, row_to_json_object, substitute_params,
};
use super::super::store::TransactionStore;
use crate::api::{TxUpdateCallback, TxUpdatePropertyCallback};
use crate::execution::callbacks::sql_generator;

/// Create the tx_update callback.
///
/// Updates an existing node's properties using SQL UPDATE.
pub fn create_tx_update<S>(store: Arc<TransactionStore<S>>) -> TxUpdateCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(
        move |tx_id: String, workspace: String, path: String, data: Value| {
            let store = store.clone();

            Box::pin(async move {
                let engine = store.get(&tx_id).ok_or_else(|| {
                    raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
                })?;

                let engine_guard = engine.lock().await;

                // Get existing node
                let get_stmt = sql_generator::generate_select_by_path(&workspace, &path);
                let get_sql = substitute_params(&get_stmt.sql, &get_stmt.params);
                let mut stream = engine_guard.execute(&get_sql).await?;

                let existing_node = if let Some(result) = stream.next().await {
                    let row = result?;
                    let obj = row_to_json_object(row);
                    let json_value = Value::Object(obj.clone());
                    tracing::debug!(
                        "tx_update: parsed row columns: {:?}, JSON: {}",
                        obj.keys().collect::<Vec<_>>(),
                        serde_json::to_string(&json_value)
                            .unwrap_or_else(|_| "<error>".to_string())
                    );
                    Some(serde_json::from_value(json_value.clone())
                    .map_err(|e| {
                        tracing::error!(
                            "tx_update: Failed to parse node from JSON. Error: {}. Path: {:?}. JSON: {}",
                            e,
                            e.classify(),
                            serde_json::to_string_pretty(&json_value).unwrap_or_else(|_| "<error>".to_string())
                        );
                        raisin_error::Error::Internal(format!("Failed to parse node: {}", e))
                    })?)
                } else {
                    None
                };

                let mut node = existing_node.ok_or_else(|| {
                    raisin_error::Error::NotFound(format!("Node not found: {}", path))
                })?;

                // Apply updates
                apply_node_updates(&mut node, data)?;

                // Execute UPDATE
                let stmt =
                    sql_generator::generate_update_properties(&workspace, &path, &node.properties);
                let final_sql = substitute_params(&stmt.sql, &stmt.params);
                let mut update_stream = engine_guard.execute(&final_sql).await?;
                while update_stream.next().await.is_some() {}

                Ok(())
            })
        },
    )
}

/// Create the tx_update_property callback.
///
/// Updates a single property using SQL UPDATE with JSON merge.
pub fn create_tx_update_property<S>(store: Arc<TransactionStore<S>>) -> TxUpdatePropertyCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(
        move |tx_id: String,
              workspace: String,
              node_path: String,
              property_path: String,
              value: Value| {
            let store = store.clone();

            Box::pin(async move {
                let engine = store.get(&tx_id).ok_or_else(|| {
                    raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
                })?;

                // Convert JSON value to PropertyValue
                let prop_value = json_to_property_value(value)?;

                let stmt = sql_generator::generate_update_single_property(
                    &workspace,
                    &node_path,
                    &property_path,
                    &prop_value,
                );
                let final_sql = substitute_params(&stmt.sql, &stmt.params);

                let engine_guard = engine.lock().await;
                let mut stream = engine_guard.execute(&final_sql).await?;
                while stream.next().await.is_some() {}

                tracing::debug!(
                    node_path = %node_path,
                    property_path = %property_path,
                    "Updated property via SQL UPDATE in transaction"
                );

                Ok(())
            })
        },
    )
}
