// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Native IMAP protocol API bindings.
//!
//! Exposes `raisin.imap.{fetchSince,listMailboxes,fetchMessage}` to server-side
//! functions. Rust owns the IMAP protocol (TLS + LOGIN + UID FETCH); functions
//! call these high-level ops. This drives the Starlark runtime (and the
//! generated python/typescript wrappers); QuickJS registers the same calls by
//! hand in `runtime/quickjs/api_imap.rs`.
//!
//! Every operation authorizes the connection's `host:port` against the
//! function's network policy before opening a socket (see
//! `api/raisindb/imap.rs`). Connection secrets are never logged.

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all IMAP method descriptors.
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // imap.fetchSince(conn, sinceUid, opts?)
        ApiMethodDescriptor {
            internal_name: "imap_fetch_since",
            js_name: "fetchSince",
            py_name: "fetch_since",
            category: "imap",
            args: vec![
                ArgSpec::new("conn", ArgType::Json),
                ArgSpec::new("sinceUid", ArgType::I64),
                ArgSpec::new("opts", ArgType::OptionalJson),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let conn = parser.json()?;
                    let since_uid = parser.i64()?;
                    let opts = parser.optional_json()?;
                    let result = api.imap_fetch_since(conn, since_uid, opts).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // imap.listMailboxes(conn)
        ApiMethodDescriptor {
            internal_name: "imap_list_mailboxes",
            js_name: "listMailboxes",
            py_name: "list_mailboxes",
            category: "imap",
            args: vec![ArgSpec::new("conn", ArgType::Json)],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let conn = parser.json()?;
                    let result = api.imap_list_mailboxes(conn).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // imap.fetchMessage(conn, uid, opts?)
        ApiMethodDescriptor {
            internal_name: "imap_fetch_message",
            js_name: "fetchMessage",
            py_name: "fetch_message",
            category: "imap",
            args: vec![
                ArgSpec::new("conn", ArgType::Json),
                ArgSpec::new("uid", ArgType::I64),
                ArgSpec::new("opts", ArgType::OptionalJson),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let conn = parser.json()?;
                    let uid = parser.i64()?;
                    let opts = parser.optional_json()?;
                    let result = api.imap_fetch_message(conn, uid, opts).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
