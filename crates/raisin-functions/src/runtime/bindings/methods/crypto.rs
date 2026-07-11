// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Crypto API bindings
//!
//! Provides UUID generation to both QuickJS (JavaScript) and Starlark (Python) runtimes.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all crypto operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // crypto.uuid() -> string (UUID v4)
        ApiMethodDescriptor {
            internal_name: "crypto_uuid",
            js_name: "uuid",
            py_name: "uuid",
            category: "crypto",
            args: vec![],
            return_type: ReturnType::String,
            invoker: |_api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move { Ok(InvokeResult::String(uuid::Uuid::new_v4().to_string())) })
            },
        },
        // crypto.verifyJwt(token, opts?) -> { valid, claims?, error? }
        //   opts: { jwks_url?, issuer?, audience?, algorithms? }
        ApiMethodDescriptor {
            internal_name: "crypto_verify_jwt",
            js_name: "verifyJwt",
            py_name: "verify_jwt",
            category: "crypto",
            args: vec![
                ArgSpec::new("token", ArgType::String),
                ArgSpec::new("opts", ArgType::OptionalJson),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let token = parser.string()?;
                    let opts = parser.optional_json()?.unwrap_or(Value::Null);
                    let result = api.crypto_verify_jwt(&token, opts).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
