// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function execution API bindings

use crate::api::{FunctionApi, FunctionExecuteContext};
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all function execution method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // functions.execute(functionPath, arguments, context)
        ApiMethodDescriptor {
            internal_name: "functions_execute",
            js_name: "execute",
            py_name: "execute",
            category: "functions",
            args: vec![
                ArgSpec::new("functionPath", ArgType::String),
                ArgSpec::new("arguments", ArgType::Json),
                ArgSpec::new("context", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let function_path = parser.string()?;
                    let arguments = parser.json()?;
                    let context_json = parser.json()?;

                    // Parse context from JSON. A missing or malformed context
                    // degrades to an empty one (long-standing JS contract:
                    // `raisin.functions.execute(path, args)` without a context
                    // executes with no tool-call linkage instead of erroring).
                    let context: FunctionExecuteContext = serde_json::from_value(context_json)
                        .unwrap_or_else(|_| FunctionExecuteContext {
                            tool_call_path: String::new(),
                            tool_call_workspace: String::new(),
                        });

                    let result = api
                        .function_execute(&function_path, arguments, context)
                        .await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // functions.call(functionPath, arguments) - Simple function-to-function call
        ApiMethodDescriptor {
            internal_name: "functions_call",
            js_name: "call",
            py_name: "call",
            category: "functions",
            args: vec![
                ArgSpec::new("functionPath", ArgType::String),
                ArgSpec::new("arguments", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let function_path = parser.string()?;
                    let arguments = parser.json()?;

                    let result = api.function_call(&function_path, arguments).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
