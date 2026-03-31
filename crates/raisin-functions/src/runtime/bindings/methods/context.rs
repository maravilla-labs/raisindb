// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Context and logging operation API bindings

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all context/logging operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // context.get() - returns the execution context
        ApiMethodDescriptor {
            internal_name: "context_get",
            js_name: "get",
            py_name: "get",
            category: "context",
            args: vec![],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let context = api.get_context();
                    Ok(InvokeResult::Json(context))
                })
            },
        },
        // log(level, message) - internal logging function
        ApiMethodDescriptor {
            internal_name: "log",
            js_name: "log",
            py_name: "log",
            category: "internal",
            args: vec![
                ArgSpec::new("level", ArgType::String),
                ArgSpec::new("message", ArgType::String),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let level = parser.string()?;
                    let message = parser.string()?;
                    api.log(&level, &message);
                    Ok(InvokeResult::Void)
                })
            },
        },
        // allowsAdminEscalation() - check if admin escalation is allowed
        ApiMethodDescriptor {
            internal_name: "allowsAdminEscalation",
            js_name: "allowsAdminEscalation",
            py_name: "allows_admin_escalation",
            category: "internal",
            args: vec![],
            return_type: ReturnType::Bool,
            invoker: |api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let result = api.allows_admin_escalation();
                    Ok(InvokeResult::Bool(result))
                })
            },
        },
    ]
}
