// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Native IMAP API registration for the QuickJS runtime.
//!
//! Registers the internal `__raisin_internal.imap_*` functions that back
//! `raisin.imap.{fetchSince,listMailboxes,fetchMessage}`. Each delegates to the
//! `FunctionApi` IMAP methods, which authorize the connection against the
//! function's network policy before opening a socket. Connection secrets are
//! never logged.
//!
//! NOTE: QuickJS uses this hand-written binding layer (not the shared bindings
//! registry, which drives the Starlark runtime). New `raisin.*` methods must be
//! registered here to be callable from JavaScript functions. JSON arguments
//! cross the boundary as JSON strings (parsed here), matching the other
//! `__raisin_internal.*` bindings.

use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;

use super::helpers::{json_error, run_async_blocking};
use crate::api::FunctionApi;

fn parse_conn(conn_json: &str) -> serde_json::Value {
    serde_json::from_str(conn_json).unwrap_or(serde_json::Value::Null)
}

fn parse_opts(opts_json: Option<String>) -> Option<serde_json::Value> {
    opts_json.and_then(|s| serde_json::from_str(&s).ok())
}

/// Register internal IMAP API functions.
pub(super) fn register_imap_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // imap_fetch_since(connJson, sinceUid, optsJson?) -> JSON string
    //   { messages, highestUid, uidvalidity } | { error }
    let api_fetch_since = api.clone();
    let fetch_since_fn = Function::new(
        ctx.clone(),
        move |conn_json: String, since_uid: i64, opts_json: Option<String>| {
            let api = api_fetch_since.clone();
            let conn = parse_conn(&conn_json);
            let opts = parse_opts(opts_json);
            let result =
                run_async_blocking(
                    async move { api.imap_fetch_since(conn, since_uid, opts).await },
                );
            match result {
                Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| "null".to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "imap_fetch_since failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("imap_fetch_since", fetch_since_fn)?;

    // imap_list_mailboxes(connJson) -> JSON string [ { name, path, flags } ] | { error }
    let api_list = api.clone();
    let list_fn = Function::new(ctx.clone(), move |conn_json: String| {
        let api = api_list.clone();
        let conn = parse_conn(&conn_json);
        let result = run_async_blocking(async move { api.imap_list_mailboxes(conn).await });
        match result {
            Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| "null".to_string()),
            Err(e) => {
                tracing::error!(error = %e, "imap_list_mailboxes failed");
                json_error(&e)
            }
        }
    })?;
    internal.set("imap_list_mailboxes", list_fn)?;

    // imap_fetch_message(connJson, uid, optsJson?) -> JSON string { ... } | { error }
    let api_fetch_msg = api.clone();
    let fetch_msg_fn = Function::new(
        ctx.clone(),
        move |conn_json: String, uid: i64, opts_json: Option<String>| {
            let api = api_fetch_msg.clone();
            let conn = parse_conn(&conn_json);
            let opts = parse_opts(opts_json);
            let result =
                run_async_blocking(async move { api.imap_fetch_message(conn, uid, opts).await });
            match result {
                Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| "null".to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "imap_fetch_message failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("imap_fetch_message", fetch_msg_fn)?;

    Ok(())
}
