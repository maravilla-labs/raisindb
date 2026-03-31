// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! AI operation API bindings

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all AI operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // ai.completion(request)
        ApiMethodDescriptor {
            internal_name: "ai_completion",
            js_name: "completion",
            py_name: "completion",
            category: "ai",
            args: vec![ArgSpec::new("request", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let request = parser.json()?;
                    let result = api.ai_completion(request).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // ai.listModels()
        ApiMethodDescriptor {
            internal_name: "ai_listModels",
            js_name: "listModels",
            py_name: "list_models",
            category: "ai",
            args: vec![],
            return_type: ReturnType::JsonArray,
            invoker: |api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let result = api.ai_list_models().await?;
                    Ok(InvokeResult::JsonArray(result))
                })
            },
        },
        // ai.getDefaultModel(useCase)
        ApiMethodDescriptor {
            internal_name: "ai_getDefaultModel",
            js_name: "getDefaultModel",
            py_name: "get_default_model",
            category: "ai",
            args: vec![ArgSpec::new("useCase", ArgType::String)],
            return_type: ReturnType::OptionalJson,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let use_case = parser.string()?;
                    let result = api.ai_get_default_model(&use_case).await?;
                    Ok(InvokeResult::OptionalString(result))
                })
            },
        },
        // ai.embed(request)
        ApiMethodDescriptor {
            internal_name: "ai_embed",
            js_name: "embed",
            py_name: "embed",
            category: "ai",
            args: vec![ArgSpec::new("request", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let request = parser.json()?;
                    let result = api.ai_embed(request).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
