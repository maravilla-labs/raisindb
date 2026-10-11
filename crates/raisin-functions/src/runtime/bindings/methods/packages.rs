// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Package build bindings (`raisin.packages.*`), for every runtime. Each takes
//! one request object: `createFromSelection(request)` starts a streaming
//! build (idempotent on `idempotency_key`), `cancel({ jobId })` stops one
//! within seconds and leaves no partial object, `status({ jobId })` reads it.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

macro_rules! packages_method {
    ($internal:literal, $js:literal, $py:literal) => {
        ApiMethodDescriptor {
            internal_name: $internal,
            js_name: $js,
            py_name: $py,
            category: "packages",
            args: vec![ArgSpec::new("request", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let request = parser.json()?;
                    let result = api.packages_call($js, request).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        }
    };
}

/// All package build method descriptors.
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        packages_method!(
            "packages_create_from_selection",
            "createFromSelection",
            "create_from_selection"
        ),
        packages_method!("packages_cancel", "cancel", "cancel"),
        packages_method!("packages_status", "status", "status"),
    ]
}
