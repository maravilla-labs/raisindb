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

//! Transaction lifecycle callbacks: begin, commit, rollback, set_actor, set_message.

use std::sync::Arc;

use futures::StreamExt;
use raisin_binary::BinaryStorage;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use tokio::sync::Mutex;

use super::store::TransactionStore;
use crate::api::{
    TxBeginCallback, TxCommitCallback, TxRollbackCallback, TxSetActorCallback, TxSetMessageCallback,
};
use crate::execution::callbacks::query_context::QueryContext;

/// Create the tx_begin callback.
///
/// Creates a new QueryEngine and executes BEGIN to start a SQL transaction.
pub fn create_tx_begin<S, B>(
    query_ctx: Arc<QueryContext<S, B>>,
    store: Arc<TransactionStore<S>>,
) -> TxBeginCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move || {
        let ctx = query_ctx.clone();
        let store = store.clone();

        Box::pin(async move {
            // Create a fresh QueryEngine
            let engine = ctx.create_engine().await?;

            // Execute BEGIN to start the SQL transaction
            let mut stream = engine.execute("BEGIN").await?;
            while stream.next().await.is_some() {}

            tracing::debug!("Started SQL transaction via BEGIN");

            // Generate transaction ID and store the engine
            let tx_id = uuid::Uuid::new_v4().to_string();
            store.insert(tx_id.clone(), Arc::new(Mutex::new(engine)));

            Ok(tx_id)
        })
    })
}

/// Create the tx_commit callback.
///
/// Executes COMMIT on the transaction's QueryEngine and removes it from the store.
pub fn create_tx_commit<S>(store: Arc<TransactionStore<S>>) -> TxCommitCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |tx_id: String| {
        let store = store.clone();

        Box::pin(async move {
            let engine = store.remove(&tx_id).ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
            })?;

            // Execute COMMIT
            let engine_guard = engine.lock().await;
            let mut stream = engine_guard.execute("COMMIT").await?;
            while stream.next().await.is_some() {}

            tracing::debug!("Committed SQL transaction via COMMIT");

            Ok(())
        })
    })
}

/// Create the tx_rollback callback.
///
/// Executes ROLLBACK on the transaction's QueryEngine and removes it from the store.
pub fn create_tx_rollback<S>(store: Arc<TransactionStore<S>>) -> TxRollbackCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |tx_id: String| {
        let store = store.clone();

        Box::pin(async move {
            let engine = store.remove(&tx_id).ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
            })?;

            // Execute ROLLBACK
            let engine_guard = engine.lock().await;
            let mut stream = engine_guard.execute("ROLLBACK").await?;
            while stream.next().await.is_some() {}

            tracing::debug!("Rolled back SQL transaction via ROLLBACK");

            Ok(())
        })
    })
}

/// Create the tx_set_actor callback.
///
/// Sets the actor for the transaction's commit metadata.
pub fn create_tx_set_actor<S>(store: Arc<TransactionStore<S>>) -> TxSetActorCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |tx_id: String, actor: String| {
        let store = store.clone();

        Box::pin(async move {
            let engine = store.get(&tx_id).ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
            })?;

            // Execute SET ACTOR
            let sql = format!("SET ACTOR = '{}'", actor.replace('\'', "''"));
            let engine_guard = engine.lock().await;
            let mut stream = engine_guard.execute(&sql).await?;
            while stream.next().await.is_some() {}

            Ok(())
        })
    })
}

/// Create the tx_set_message callback.
///
/// Sets the commit message for the transaction.
pub fn create_tx_set_message<S>(store: Arc<TransactionStore<S>>) -> TxSetMessageCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |tx_id: String, message: String| {
        let store = store.clone();

        Box::pin(async move {
            let engine = store.get(&tx_id).ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Transaction not found: {}", tx_id))
            })?;

            // Execute SET MESSAGE
            let sql = format!("SET MESSAGE = '{}'", message.replace('\'', "''"));
            let engine_guard = engine.lock().await;
            let mut stream = engine_guard.execute(&sql).await?;
            while stream.next().await.is_some() {}

            Ok(())
        })
    })
}
