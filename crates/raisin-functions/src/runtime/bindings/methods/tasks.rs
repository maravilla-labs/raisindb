// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Task operation API bindings

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all task operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // tasks.create(request)
        ApiMethodDescriptor {
            internal_name: "tasks_create",
            js_name: "create",
            py_name: "create",
            category: "tasks",
            args: vec![ArgSpec::new("request", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let request = parser.json()?;
                    let result = api.task_create(request).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // tasks.update(taskId, updates)
        ApiMethodDescriptor {
            internal_name: "tasks_update",
            js_name: "update",
            py_name: "update",
            category: "tasks",
            args: vec![
                ArgSpec::new("task_id", ArgType::String),
                ArgSpec::new("updates", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let task_id = parser.string()?;
                    let updates = parser.json()?;
                    let result = api.task_update(&task_id, updates).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // tasks.complete(taskId, response)
        ApiMethodDescriptor {
            internal_name: "tasks_complete",
            js_name: "complete",
            py_name: "complete",
            category: "tasks",
            args: vec![
                ArgSpec::new("task_id", ArgType::String),
                ArgSpec::new("response", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let task_id = parser.string()?;
                    let response = parser.json()?;
                    let result = api.task_complete(&task_id, response).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // tasks.query(query)
        ApiMethodDescriptor {
            internal_name: "tasks_query",
            js_name: "query",
            py_name: "query",
            category: "tasks",
            args: vec![ArgSpec::new("query", ArgType::Json)],
            return_type: ReturnType::JsonArray,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let query = parser.json()?;
                    let result = api.task_query(query).await?;
                    Ok(InvokeResult::JsonArray(result))
                })
            },
        },
    ]
}
