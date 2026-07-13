// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Integration / mount operation API bindings.
//!
//! Exposes `raisin.integrations.sync_now(mountId, mode?)` to server-side
//! functions: enqueues a deduped `VirtualMountSync` job for a virtual mount
//! (connector). Both the Starlark and QuickJS runtimes consume this single
//! definition through their registry gateways.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all integration / mount method descriptors.
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // integrations.sync_now(mountId, mode?)
        ApiMethodDescriptor {
            internal_name: "integrations_sync_now",
            js_name: "syncNow",
            py_name: "sync_now",
            category: "integrations",
            args: vec![
                ArgSpec::new("mountId", ArgType::String),
                ArgSpec::new("mode", ArgType::OptionalString),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let mount_id = parser.string()?;
                    let mode = parser.optional_string()?;
                    let result = api
                        .integrations_sync_now(&mount_id, mode.as_deref())
                        .await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
