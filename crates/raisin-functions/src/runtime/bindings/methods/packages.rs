// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Package build bindings, for every runtime (registry names are the
//! contract Studio's dispatcher calls):
//!
//! - `packages_capabilities []` → `{streaming_build, record_mirror, sidecars,
//!   ranged_download, cancel, idempotency, excluded_paths, version}`
//! - `packages_createFromSelection [request]` → `{job_id, existing}`
//! - `packages_cancel [job_id]` → `{job_id, cancelled}`
//! - `packages_status [job_id]` → `{job_id, status, progress, error, result}` | null
//! - `packages_deleteArtifacts [{workspace, path}]` → `{deleted}`
//! - `resources_readRange [storage_key, offset, len]` → `{base64, bytes, …}`
//!
//! See `raisin_rocksdb::package_build::call` for the scoping and actor rules.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::{json, Value};
use std::sync::Arc;

/// All package build method descriptors.
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        ApiMethodDescriptor {
            internal_name: "packages_capabilities",
            js_name: "capabilities",
            py_name: "capabilities",
            category: "packages",
            args: vec![],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let result = api.packages_call("capabilities", Value::Null).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        ApiMethodDescriptor {
            internal_name: "packages_createFromSelection",
            js_name: "createFromSelection",
            py_name: "create_from_selection",
            category: "packages",
            args: vec![ArgSpec::new("request", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let request = parser.json()?;
                    let result = api.packages_call("createFromSelection", request).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        ApiMethodDescriptor {
            internal_name: "packages_cancel",
            js_name: "cancel",
            py_name: "cancel",
            category: "packages",
            args: vec![ArgSpec::new("jobId", ArgType::String)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let job_id = parser.string()?;
                    let result = api.packages_call("cancel", Value::String(job_id)).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        ApiMethodDescriptor {
            internal_name: "packages_status",
            js_name: "status",
            py_name: "status",
            category: "packages",
            args: vec![ArgSpec::new("jobId", ArgType::String)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let job_id = parser.string()?;
                    let result = api.packages_call("status", Value::String(job_id)).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        ApiMethodDescriptor {
            internal_name: "packages_deleteArtifacts",
            js_name: "deleteArtifacts",
            py_name: "delete_artifacts",
            category: "packages",
            args: vec![ArgSpec::new("record", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let record = parser.json()?;
                    let result = api.packages_call("deleteArtifacts", record).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        ApiMethodDescriptor {
            internal_name: "resources_readRange",
            js_name: "readRange",
            py_name: "read_range",
            category: "resources",
            args: vec![
                ArgSpec::new("storageKey", ArgType::String),
                ArgSpec::new("offset", ArgType::I64),
                ArgSpec::new("len", ArgType::I64),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let storage_key = parser.string()?;
                    let offset = parser.i64()?.max(0);
                    let len = parser.i64()?.max(0);
                    let result = api
                        .packages_call(
                            "readRange",
                            json!({ "storage_key": storage_key, "offset": offset, "len": len }),
                        )
                        .await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
