// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Node operation callback type definitions

use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Callback for node get operations
pub type NodeGetCallback = Arc<
    dyn Fn(
            String, // workspace
            String, // path
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Value>>> + Send>>
        + Send
        + Sync,
>;

/// Callback for node get by ID operations
pub type NodeGetByIdCallback = Arc<
    dyn Fn(
            String, // workspace
            String, // id
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Value>>> + Send>>
        + Send
        + Sync,
>;

/// Callback for node creation
pub type NodeCreateCallback = Arc<
    dyn Fn(
            String, // workspace
            String, // parent_path
            Value,  // data
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for node update
pub type NodeUpdateCallback = Arc<
    dyn Fn(
            String, // workspace
            String, // path
            Value,  // data
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for node deletion
pub type NodeDeleteCallback = Arc<
    dyn Fn(
            String, // workspace
            String, // path
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback for updating a specific property by path
///
/// This allows efficient updates to individual properties without fetching/replacing
/// the entire node. The property_path uses dot notation (e.g., "user.address.city").
pub type NodeUpdatePropertyCallback = Arc<
    dyn Fn(
            String, // workspace
            String, // node_path
            String, // property_path (dot notation, e.g., "user.email")
            Value,  // value
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Callback for moving a node to a new parent path
///
/// Moves a node and all its descendants to a new location.
/// The node's name is preserved; only the parent changes.
pub type NodeMoveCallback = Arc<
    dyn Fn(
            String, // workspace
            String, // node_path (current path)
            String, // new_parent_path
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for node query
pub type NodeQueryCallback = Arc<
    dyn Fn(
            String, // workspace
            Value,  // query
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Value>>> + Send>>
        + Send
        + Sync,
>;

/// Callback for getting node children
pub type NodeGetChildrenCallback = Arc<
    dyn Fn(
            String,      // workspace
            String,      // parent_path
            Option<u32>, // limit
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Value>>> + Send>>
        + Send
        + Sync,
>;
