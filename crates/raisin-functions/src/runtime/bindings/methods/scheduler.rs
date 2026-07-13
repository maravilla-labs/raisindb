// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Scheduled invocation API bindings (`raisin.scheduler.*`)

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all scheduler method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // scheduler.schedule(request) - schedule a one-shot invocation of a
        // function or flow. request: { targetKind, targetPath, input?,
        // runAt (RFC3339), externalKey?, branch?, workspace?, maxRetries? }.
        // Returns { job_id, invocation_id, status: "scheduled", run_at }.
        ApiMethodDescriptor {
            internal_name: "scheduler_schedule",
            js_name: "schedule",
            py_name: "schedule",
            category: "scheduler",
            args: vec![ArgSpec::new("request", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let request = parser.json()?;

                    let result = api.scheduler_schedule(request).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // scheduler.cancel(jobIdOrKey) - cancel a pending invocation by job
        // id or external key. Returns { job_id, status: "cancelled" }.
        ApiMethodDescriptor {
            internal_name: "scheduler_cancel",
            js_name: "cancel",
            py_name: "cancel",
            category: "scheduler",
            args: vec![ArgSpec::new("jobIdOrKey", ArgType::String)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let job_id_or_key = parser.string()?;

                    let result = api.scheduler_cancel(&job_id_or_key).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // scheduler.list(filter?) - list this repository's scheduled
        // invocations. filter: { externalKey?, status? }.
        // Returns { invocations: [...] }.
        ApiMethodDescriptor {
            internal_name: "scheduler_list",
            js_name: "list",
            py_name: "list",
            category: "scheduler",
            args: vec![ArgSpec::new("filter", ArgType::OptionalJson)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let filter = parser
                        .optional_json()?
                        .unwrap_or_else(|| serde_json::json!({}));

                    let result = api.scheduler_list(filter).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // scheduler.get(jobIdOrKey) - fetch a single scheduled invocation by
        // job id or external key.
        ApiMethodDescriptor {
            internal_name: "scheduler_get",
            js_name: "get",
            py_name: "get",
            category: "scheduler",
            args: vec![ArgSpec::new("jobIdOrKey", ArgType::String)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let job_id_or_key = parser.string()?;

                    let result = api.scheduler_get(&job_id_or_key).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
