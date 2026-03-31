// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Event operation API bindings

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all event operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // events.emit(eventType, data)
        ApiMethodDescriptor {
            internal_name: "events_emit",
            js_name: "emit",
            py_name: "emit",
            category: "events",
            args: vec![
                ArgSpec::new("eventType", ArgType::String),
                ArgSpec::new("data", ArgType::Json),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let event_type = parser.string()?;
                    let data = parser.json()?;
                    api.emit_event(&event_type, data).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
    ]
}
