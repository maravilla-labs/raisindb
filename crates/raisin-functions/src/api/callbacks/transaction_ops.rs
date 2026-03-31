// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Transaction operation callback type definitions

use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Callback to begin a transaction, returns transaction ID
pub type TxBeginCallback = Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>>
        + Send
        + Sync,
>;

/// Callback to commit a transaction
pub type TxCommitCallback = Arc<
    dyn Fn(
            String, // tx_id
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback to rollback a transaction
pub type TxRollbackCallback = Arc<
    dyn Fn(
            String, // tx_id
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback to set actor for transaction
pub type TxSetActorCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // actor
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback to set message for transaction
pub type TxSetMessageCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // message
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node create (auto-generates ID, builds path from parent+name)
pub type TxCreateCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            String, // parent_path
            Value,  // data
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node add (uses provided path, auto-generates ID if missing)
pub type TxAddCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            Value,  // data (must include path)
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node put (create or update by ID)
pub type TxPutCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            Value,  // data (must include path, auto-generates ID if missing)
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node upsert (create or update by PATH)
pub type TxUpsertCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            Value,  // data (must include path, auto-generates ID if missing)
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node create with deep parent creation
pub type TxCreateDeepCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            String, // parent_path
            Value,  // data
            String, // parent_node_type (e.g., "raisin:Folder")
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node upsert with deep parent creation
pub type TxUpsertDeepCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            Value,  // data (must include path)
            String, // parent_node_type (e.g., "raisin:Folder")
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node update (updates existing node by path)
pub type TxUpdateCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            String, // path
            Value,  // data
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node delete by path
pub type TxDeleteCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            String, // path
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node delete by ID
pub type TxDeleteByIdCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            String, // id
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node get by ID
pub type TxGetCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            String, // id
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Value>>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node get by path
pub type TxGetByPathCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            String, // path
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Value>>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional list children
pub type TxListChildrenCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            String, // parent_path
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Value>>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional node move
pub type TxMoveCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            String, // node_path
            String, // new_parent_path
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for transactional property update
pub type TxUpdatePropertyCallback = Arc<
    dyn Fn(
            String, // tx_id
            String, // workspace
            String, // node_path
            String, // property_path
            Value,  // value
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;
