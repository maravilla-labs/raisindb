// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Crypto API bindings
//!
//! Provides UUID generation to both QuickJS (JavaScript) and Starlark (Python) runtimes.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{ApiMethodDescriptor, InvokeResult, ReturnType};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all crypto operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // crypto.uuid() -> string (UUID v4)
        ApiMethodDescriptor {
            internal_name: "crypto_uuid",
            js_name: "uuid",
            py_name: "uuid",
            category: "crypto",
            args: vec![],
            return_type: ReturnType::String,
            invoker: |_api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move { Ok(InvokeResult::String(uuid::Uuid::new_v4().to_string())) })
            },
        },
    ]
}
