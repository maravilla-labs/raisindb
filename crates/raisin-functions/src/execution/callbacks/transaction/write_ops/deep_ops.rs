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

//! Deep transaction write callbacks with automatic parent creation.
//!
//! These callbacks handle create_deep and upsert_deep operations
//! that automatically create parent folder nodes when they do not exist.

use std::sync::Arc;

use futures::StreamExt;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::Value;

use super::super::helpers::{parse_node_create_data, parse_node_full_data, substitute_params};
use super::super::store::TransactionStore;
use crate::api::{TxCreateDeepCallback, TxUpsertDeepCallback};
use crate::execution::callbacks::sql_generator;

/// Create the tx_create_deep callback.
///
/// Creates a new node with auto-created parent folders.
/// NOTE: Deep creation requires multiple SQL statements - creates parents first.
pub fn create_tx_create_deep<S>(store: Arc<TransactionStore<S>>) -> TxCreateDeepCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(
        move |tx_id: String,
              workspace: String,
              parent_path: String,
              data: Value,
              parent_node_type: String| {
            let store = store.clone();

            Box::pin(async move {
                let engine = store.get(&tx_id).ok_or_else(|| {
                    raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
                })?;

                let node = parse_node_create_data(&parent_path, data)?;

                // Create parent folders if needed
                let parts: Vec<&str> = parent_path.split('/').filter(|s| !s.is_empty()).collect();
                let mut current_path = String::new();

                let engine_guard = engine.lock().await;

                for part in parts {
                    current_path = format!("{}/{}", current_path, part);

                    // Check if parent exists
                    let check_stmt =
                        sql_generator::generate_select_by_path(&workspace, &current_path);
                    let check_sql = substitute_params(&check_stmt.sql, &check_stmt.params);
                    let mut stream = engine_guard.execute(&check_sql).await?;

                    let mut exists = false;
                    while let Some(result) = stream.next().await {
                        if result.is_ok() {
                            exists = true;
                            break;
                        }
                    }

                    if !exists {
                        // Create parent folder
                        let parent_node = Node {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: part.to_string(),
                            path: current_path.clone(),
                            node_type: parent_node_type.clone(),
                            created_at: Some(chrono::Utc::now()),
                            ..Default::default()
                        };
                        let insert_stmt = sql_generator::generate_insert(&workspace, &parent_node);
                        let insert_sql = substitute_params(&insert_stmt.sql, &insert_stmt.params);
                        let mut insert_stream = engine_guard.execute(&insert_sql).await?;
                        while insert_stream.next().await.is_some() {}
                    }
                }

                // Create the actual node
                let stmt = sql_generator::generate_insert(&workspace, &node);
                let final_sql = substitute_params(&stmt.sql, &stmt.params);
                let mut stream = engine_guard.execute(&final_sql).await?;
                while stream.next().await.is_some() {}

                Ok(serde_json::to_value(node).unwrap_or_default())
            })
        },
    )
}

/// Create the tx_upsert_deep callback.
///
/// Creates or updates a node by PATH with auto-created parent folders.
pub fn create_tx_upsert_deep<S>(store: Arc<TransactionStore<S>>) -> TxUpsertDeepCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(
        move |tx_id: String, workspace: String, data: Value, parent_node_type: String| {
            let store = store.clone();

            Box::pin(async move {
                let engine = store.get(&tx_id).ok_or_else(|| {
                    raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
                })?;

                let node = parse_node_full_data(data)?;

                // Get parent path from node path
                let parent_path = node.path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
                if !parent_path.is_empty() {
                    // Create parent folders if needed
                    let parts: Vec<&str> =
                        parent_path.split('/').filter(|s| !s.is_empty()).collect();
                    let mut current_path = String::new();

                    let engine_guard = engine.lock().await;

                    for part in parts {
                        current_path = format!("{}/{}", current_path, part);

                        // Check if parent exists
                        let check_stmt =
                            sql_generator::generate_select_by_path(&workspace, &current_path);
                        let check_sql = substitute_params(&check_stmt.sql, &check_stmt.params);
                        let mut stream = engine_guard.execute(&check_sql).await?;

                        let mut exists = false;
                        while let Some(result) = stream.next().await {
                            if result.is_ok() {
                                exists = true;
                                break;
                            }
                        }

                        if !exists {
                            // Create parent folder
                            let parent_node = Node {
                                id: uuid::Uuid::new_v4().to_string(),
                                name: part.to_string(),
                                path: current_path.clone(),
                                node_type: parent_node_type.clone(),
                                created_at: Some(chrono::Utc::now()),
                                ..Default::default()
                            };
                            let insert_stmt =
                                sql_generator::generate_insert(&workspace, &parent_node);
                            let insert_sql =
                                substitute_params(&insert_stmt.sql, &insert_stmt.params);
                            let mut insert_stream = engine_guard.execute(&insert_sql).await?;
                            while insert_stream.next().await.is_some() {}
                        }
                    }

                    // Upsert the actual node
                    let stmt = sql_generator::generate_upsert(&workspace, &node);
                    let final_sql = substitute_params(&stmt.sql, &stmt.params);
                    let mut stream = engine_guard.execute(&final_sql).await?;
                    while stream.next().await.is_some() {}
                } else {
                    // No parent needed, just upsert
                    let engine_guard = engine.lock().await;
                    let stmt = sql_generator::generate_upsert(&workspace, &node);
                    let final_sql = substitute_params(&stmt.sql, &stmt.params);
                    let mut stream = engine_guard.execute(&final_sql).await?;
                    while stream.next().await.is_some() {}
                }

                Ok(())
            })
        },
    )
}
