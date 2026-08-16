// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Transactional email API bindings.
//!
//! Exposes `raisin.email.send(message)` to server-side functions. Both the
//! Starlark and QuickJS runtimes consume this single definition through their
//! registry gateways.
//!
//! Only the message is an argument: the sender identity and the provider
//! credential come from the tenant's `/config/email` node, and the send is
//! authorized against that config and the function's secret policy before any
//! socket is opened (see `api/raisindb/email.rs`).

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all email method descriptors.
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // email.send(message)
        ApiMethodDescriptor {
            internal_name: "email_send",
            js_name: "send",
            py_name: "send",
            category: "email",
            args: vec![ArgSpec::new("message", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let message = parser.json()?;
                    let result = api.email_send(message).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
