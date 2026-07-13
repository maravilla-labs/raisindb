// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! HTTP operation API bindings

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Wall-clock timeout for external HTTP requests (2 minutes).
///
/// Applied inside the shared invoker so BOTH runtimes (QuickJS `http.fetch`,
/// Starlark `http.request`) are protected against an unresponsive target
/// blocking a function execution slot indefinitely.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Get all HTTP operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // Internal: http.request(method, url, options) - used by wrapper to implement fetch/get/post/etc
        ApiMethodDescriptor {
            internal_name: "http_request",
            js_name: "request",
            py_name: "request",
            category: "http",
            args: vec![
                ArgSpec::new("method", ArgType::String),
                ArgSpec::new("url", ArgType::String),
                ArgSpec::new("options", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let method = parser.string()?;
                    let url = parser.string()?;
                    let options = parser.json()?;
                    let result =
                        tokio::time::timeout(FETCH_TIMEOUT, api.http_request(&method, &url, options))
                            .await
                            .map_err(|_| {
                                raisin_error::Error::Internal(format!(
                                    "HTTP request timed out after {}s",
                                    FETCH_TIMEOUT.as_secs()
                                ))
                            })??;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // Convenience methods implemented via wrapper code:
        // - http.get(url, options)
        // - http.post(url, options)
        // - http.put(url, options)
        // - http.patch(url, options)
        // - http.delete(url, options)
        //
        // These are generated in the JavaScript/Python wrappers rather than
        // being separate bindings, to avoid code duplication.
    ]
}
