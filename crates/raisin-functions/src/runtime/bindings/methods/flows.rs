// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow execution API bindings (`raisin.flows.*`)

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all flow method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // flows.run(flowPath, input) - start a raisin:Flow, fire-and-forget.
        // Returns { instance_id, job_id, status: "queued" }.
        ApiMethodDescriptor {
            internal_name: "flows_run",
            js_name: "run",
            py_name: "run",
            category: "flows",
            args: vec![
                ArgSpec::new("flowPath", ArgType::String),
                ArgSpec::new("input", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let flow_path = parser.string()?;
                    let input = parser.json()?;

                    let result = api.flow_run(&flow_path, input).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
