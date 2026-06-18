// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Lock / inventory operation API bindings
//!
//! Exposes `raisin.locks.*` (lease-locks with fencing tokens) and
//! `raisin.inventory.*` (counting reservations) to server-side functions.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all lock / inventory method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // locks.acquire(key, ttlMs, owner?)
        ApiMethodDescriptor {
            internal_name: "locks_acquire",
            js_name: "acquire",
            py_name: "acquire",
            category: "locks",
            args: vec![
                ArgSpec::new("key", ArgType::String),
                ArgSpec::new("ttlMs", ArgType::I64),
                ArgSpec::new("owner", ArgType::OptionalString),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let key = parser.string()?;
                    let ttl_ms = parser.i64()?;
                    let owner = parser.optional_string()?;
                    let result = api.lock_acquire(&key, ttl_ms, owner).await?;
                    // Discriminated shape, consistent across all surfaces.
                    let value = match result {
                        Some(mut v) => {
                            if let Some(obj) = v.as_object_mut() {
                                obj.insert("acquired".to_string(), Value::Bool(true));
                            }
                            v
                        }
                        None => serde_json::json!({ "acquired": false }),
                    };
                    Ok(InvokeResult::Json(value))
                })
            },
        },
        // locks.release(key, token)
        ApiMethodDescriptor {
            internal_name: "locks_release",
            js_name: "release",
            py_name: "release",
            category: "locks",
            args: vec![
                ArgSpec::new("key", ArgType::String),
                ArgSpec::new("token", ArgType::I64),
            ],
            return_type: ReturnType::Bool,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let key = parser.string()?;
                    let token = parser.i64()?;
                    let result = api.lock_release(&key, token).await?;
                    Ok(InvokeResult::Bool(result))
                })
            },
        },
        // locks.renew(key, token, ttlMs)
        ApiMethodDescriptor {
            internal_name: "locks_renew",
            js_name: "renew",
            py_name: "renew",
            category: "locks",
            args: vec![
                ArgSpec::new("key", ArgType::String),
                ArgSpec::new("token", ArgType::I64),
                ArgSpec::new("ttlMs", ArgType::I64),
            ],
            return_type: ReturnType::Bool,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let key = parser.string()?;
                    let token = parser.i64()?;
                    let ttl_ms = parser.i64()?;
                    let result = api.lock_renew(&key, token, ttl_ms).await?;
                    Ok(InvokeResult::Bool(result))
                })
            },
        },
        // inventory.claim(pool, n, capacity)
        ApiMethodDescriptor {
            internal_name: "inventory_claim",
            js_name: "claim",
            py_name: "claim",
            category: "inventory",
            args: vec![
                ArgSpec::new("pool", ArgType::String),
                ArgSpec::new("n", ArgType::I64),
                ArgSpec::new("capacity", ArgType::I64),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let pool = parser.string()?;
                    let n = parser.i64()?;
                    let capacity = parser.i64()?;
                    let result = api.inventory_claim(&pool, n, capacity).await?;
                    // Discriminated shape, consistent across all surfaces.
                    let value = match result {
                        Some(mut v) => {
                            if let Some(obj) = v.as_object_mut() {
                                obj.insert("claimed".to_string(), Value::Bool(true));
                            }
                            v
                        }
                        None => serde_json::json!({ "claimed": false }),
                    };
                    Ok(InvokeResult::Json(value))
                })
            },
        },
        // inventory.release(pool, n)
        ApiMethodDescriptor {
            internal_name: "inventory_release",
            js_name: "release",
            py_name: "release",
            category: "inventory",
            args: vec![
                ArgSpec::new("pool", ArgType::String),
                ArgSpec::new("n", ArgType::I64),
            ],
            return_type: ReturnType::I64,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let pool = parser.string()?;
                    let n = parser.i64()?;
                    let result = api.inventory_release(&pool, n).await?;
                    Ok(InvokeResult::I64(result))
                })
            },
        },
    ]
}
