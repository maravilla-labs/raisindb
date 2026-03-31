// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Date/time operation API bindings
//!
//! Provides chrono-based datetime functionality to both QuickJS (JavaScript)
//! and Starlark (Python) runtimes.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all date/time operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // date.now() -> string (ISO 8601 UTC datetime)
        ApiMethodDescriptor {
            internal_name: "date_now",
            js_name: "now",
            py_name: "now",
            category: "date",
            args: vec![],
            return_type: ReturnType::String,
            invoker: |api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let result = api.date_now();
                    Ok(InvokeResult::String(result))
                })
            },
        },
        // date.timestamp() -> i64 (Unix timestamp in seconds)
        ApiMethodDescriptor {
            internal_name: "date_timestamp",
            js_name: "timestamp",
            py_name: "timestamp",
            category: "date",
            args: vec![],
            return_type: ReturnType::I64,
            invoker: |api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let result = api.date_timestamp();
                    Ok(InvokeResult::I64(result))
                })
            },
        },
        // date.timestampMillis() -> i64 (Unix timestamp in milliseconds)
        ApiMethodDescriptor {
            internal_name: "date_timestampMillis",
            js_name: "timestampMillis",
            py_name: "timestamp_millis",
            category: "date",
            args: vec![],
            return_type: ReturnType::I64,
            invoker: |api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let result = api.date_timestamp_millis();
                    Ok(InvokeResult::I64(result))
                })
            },
        },
        // date.parse(dateStr, format?) -> i64 (parse date string to Unix timestamp)
        ApiMethodDescriptor {
            internal_name: "date_parse",
            js_name: "parse",
            py_name: "parse",
            category: "date",
            args: vec![
                ArgSpec::new("dateStr", ArgType::String),
                ArgSpec::new("format", ArgType::OptionalString),
            ],
            return_type: ReturnType::I64,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let date_str = parser.string()?;
                    let format = parser.optional_string()?;
                    let result = api.date_parse(&date_str, format.as_deref())?;
                    Ok(InvokeResult::I64(result))
                })
            },
        },
        // date.format(timestamp, format?) -> string (format timestamp to string)
        ApiMethodDescriptor {
            internal_name: "date_format",
            js_name: "format",
            py_name: "format",
            category: "date",
            args: vec![
                ArgSpec::new("timestamp", ArgType::I64),
                ArgSpec::new("format", ArgType::OptionalString),
            ],
            return_type: ReturnType::String,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let timestamp = parser.i64()?;
                    let format = parser.optional_string()?;
                    let result = api.date_format(timestamp, format.as_deref())?;
                    Ok(InvokeResult::String(result))
                })
            },
        },
        // date.addDays(timestamp, days) -> i64 (add days to timestamp)
        ApiMethodDescriptor {
            internal_name: "date_addDays",
            js_name: "addDays",
            py_name: "add_days",
            category: "date",
            args: vec![
                ArgSpec::new("timestamp", ArgType::I64),
                ArgSpec::new("days", ArgType::I64),
            ],
            return_type: ReturnType::I64,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let timestamp = parser.i64()?;
                    let days = parser.i64()?;
                    let result = api.date_add_days(timestamp, days)?;
                    Ok(InvokeResult::I64(result))
                })
            },
        },
        // date.diffDays(ts1, ts2) -> i64 (difference in days between timestamps)
        ApiMethodDescriptor {
            internal_name: "date_diffDays",
            js_name: "diffDays",
            py_name: "diff_days",
            category: "date",
            args: vec![
                ArgSpec::new("ts1", ArgType::I64),
                ArgSpec::new("ts2", ArgType::I64),
            ],
            return_type: ReturnType::I64,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let ts1 = parser.i64()?;
                    let ts2 = parser.i64()?;
                    let result = api.date_diff_days(ts1, ts2);
                    Ok(InvokeResult::I64(result))
                })
            },
        },
    ]
}
