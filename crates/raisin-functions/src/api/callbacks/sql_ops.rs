// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! SQL operation callback type definitions

use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Callback for SQL query
pub type SqlQueryCallback = Arc<
    dyn Fn(
            String,     // sql
            Vec<Value>, // params
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for SQL execute
pub type SqlExecuteCallback = Arc<
    dyn Fn(
            String,     // sql
            Vec<Value>, // params
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<i64>> + Send>>
        + Send
        + Sync,
>;
