// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! SQL operation API bindings

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all SQL operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // sql.query(sql, params)
        ApiMethodDescriptor {
            internal_name: "sql_query",
            js_name: "query",
            py_name: "query",
            category: "sql",
            args: vec![
                ArgSpec::new("sql", ArgType::String),
                ArgSpec::new("params", ArgType::JsonArray),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let sql = parser.string()?;
                    let params = parser.json_array()?;
                    let result = api.sql_query(&sql, params).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // sql.execute(sql, params)
        ApiMethodDescriptor {
            internal_name: "sql_execute",
            js_name: "execute",
            py_name: "execute",
            category: "sql",
            args: vec![
                ArgSpec::new("sql", ArgType::String),
                ArgSpec::new("params", ArgType::JsonArray),
            ],
            return_type: ReturnType::I64,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let sql = parser.string()?;
                    let params = parser.json_array()?;
                    let result = api.sql_execute(&sql, params).await?;
                    Ok(InvokeResult::I64(result))
                })
            },
        },
    ]
}
