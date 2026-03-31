// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Resource operation API bindings

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all resource operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // resources.getBinary(storageKey)
        ApiMethodDescriptor {
            internal_name: "resources_getBinary",
            js_name: "getBinary",
            py_name: "get_binary",
            category: "resources",
            args: vec![ArgSpec::new("storageKey", ArgType::String)],
            return_type: ReturnType::String,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let storage_key = parser.string()?;
                    let result = api.resource_get_binary(&storage_key).await?;
                    Ok(InvokeResult::String(result))
                })
            },
        },
    ]
}
