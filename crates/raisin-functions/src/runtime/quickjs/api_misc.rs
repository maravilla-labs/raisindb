// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Miscellaneous API registration for the QuickJS runtime.
//!
//! Registers internal functions for SQL, HTTP, events, AI, functions,
//! tasks, and crypto operations.

use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;

use super::helpers::{
    json_error, json_error_with_fields, run_async_blocking, run_async_blocking_with_timeout,
    FETCH_TIMEOUT,
};
use crate::api::FunctionApi;

/// Register internal SQL API functions.
pub(super) fn register_sql_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // sql_query
    let api_query = api.clone();
    let query_fn = Function::new(
        ctx.clone(),
        move |sql_str: String, params_json: Option<String>| {
            let api = api_query.clone();
            let params_vec: Vec<serde_json::Value> = params_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let result =
                run_async_blocking(async move { api.sql_query(&sql_str, params_vec).await });
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed","rows":[]}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "sql_query failed");
                    json_error_with_fields(&e, serde_json::json!({"rows": []}))
                }
            }
        },
    )?;
    internal.set("sql_query", query_fn)?;

    // sql_execute
    let api_execute = api.clone();
    let execute_fn = Function::new(
        ctx.clone(),
        move |sql_str: String, params_json: Option<String>| {
            let api = api_execute.clone();
            let params_vec: Vec<serde_json::Value> = params_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let result =
                run_async_blocking(async move { api.sql_execute(&sql_str, params_vec).await });
            match result {
                Ok(count) => count,
                Err(e) => {
                    tracing::error!(error = %e, "sql_execute failed");
                    -1
                }
            }
        },
    )?;
    internal.set("sql_execute", execute_fn)?;

    Ok(())
}

/// Register internal HTTP API functions.
pub(super) fn register_http_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // http_fetch
    let api_fetch = api.clone();
    let fetch_fn = Function::new(
        ctx.clone(),
        move |url: String, options_json: Option<String>| {
            let api = api_fetch.clone();
            let opts: serde_json::Value = options_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::json!({}));
            let method = opts
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("GET")
                .to_string();
            let result = run_async_blocking_with_timeout(
                async move { api.http_request(&method, &url, opts).await },
                FETCH_TIMEOUT,
            )
            .and_then(|r| r);
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed","status":0}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "http_fetch failed");
                    json_error_with_fields(&e, serde_json::json!({"status": 0, "ok": false}))
                }
            }
        },
    )?;
    internal.set("http_fetch", fetch_fn)?;

    Ok(())
}

/// Register internal events API functions.
pub(super) fn register_events_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // events_emit
    let api_emit = api.clone();
    let emit_fn = Function::new(ctx.clone(), move |event_type: String, data_json: String| {
        let api = api_emit.clone();
        let data: serde_json::Value =
            serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
        let result = run_async_blocking(async move { api.emit_event(&event_type, data).await });
        match result {
            Ok(()) => true,
            Err(e) => {
                tracing::error!(error = %e, "emit_event failed");
                false
            }
        }
    })?;
    internal.set("events_emit", emit_fn)?;

    Ok(())
}

/// Register internal AI API functions.
pub(super) fn register_ai_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // ai_completion
    let api_completion = api.clone();
    let completion_fn = Function::new(ctx.clone(), move |request_json: String| {
        let api = api_completion.clone();
        let request: serde_json::Value =
            serde_json::from_str(&request_json).unwrap_or(serde_json::json!({}));
        let result = run_async_blocking(async move { api.ai_completion(request).await });
        match result {
            Ok(v) => serde_json::to_string(&v)
                .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
            Err(e) => {
                tracing::error!(error = %e, "ai_completion failed");
                json_error(&e)
            }
        }
    })?;
    internal.set("ai_completion", completion_fn)?;

    // ai_listModels
    let api_list = api.clone();
    let list_fn = Function::new(ctx.clone(), move || {
        let api = api_list.clone();
        let result = run_async_blocking(async move { api.ai_list_models().await });
        match result {
            Ok(v) => serde_json::to_string(&v).unwrap_or("[]".to_string()),
            Err(e) => {
                tracing::error!(error = %e, "ai_list_models failed");
                "[]".to_string()
            }
        }
    })?;
    internal.set("ai_listModels", list_fn)?;

    // ai_getDefaultModel
    let api_default = api.clone();
    let default_fn = Function::new(ctx.clone(), move |use_case: String| {
        let api = api_default.clone();
        let result = run_async_blocking(async move { api.ai_get_default_model(&use_case).await });
        match result {
            Ok(Some(model)) => model,
            Ok(None) => String::new(),
            Err(e) => {
                tracing::error!(error = %e, "ai_get_default_model failed");
                String::new()
            }
        }
    })?;
    internal.set("ai_getDefaultModel", default_fn)?;

    // ai_embed
    let api_embed = api.clone();
    let embed_fn = Function::new(ctx.clone(), move |request_json: String| {
        let api = api_embed.clone();
        let request: serde_json::Value =
            serde_json::from_str(&request_json).unwrap_or(serde_json::json!({}));
        let result = run_async_blocking(async move { api.ai_embed(request).await });
        match result {
            Ok(v) => serde_json::to_string(&v)
                .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
            Err(e) => {
                tracing::error!(error = %e, "ai_embed failed");
                json_error(&e)
            }
        }
    })?;
    internal.set("ai_embed", embed_fn)?;

    Ok(())
}

/// Register internal functions API (function-to-function calls).
pub(super) fn register_functions_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    use crate::api::FunctionExecuteContext;

    // functions_execute
    let api_execute = api.clone();
    let execute_fn = Function::new(
        ctx.clone(),
        move |function_path: String, args_json: String, context_json: String| {
            let api = api_execute.clone();

            // Parse arguments
            let arguments: serde_json::Value =
                serde_json::from_str(&args_json).unwrap_or(serde_json::json!({}));

            // Parse context
            let context: FunctionExecuteContext = serde_json::from_str(&context_json)
                .unwrap_or_else(|_| FunctionExecuteContext {
                    tool_call_path: String::new(),
                    tool_call_workspace: String::new(),
                });

            let result = run_async_blocking(async move {
                api.function_execute(&function_path, arguments, context)
                    .await
            });

            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "function_execute failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("functions_execute", execute_fn)?;

    // functions_call (simple function-to-function call, no context needed)
    let api_call = api.clone();
    let call_fn = Function::new(
        ctx.clone(),
        move |function_path: String, args_json: String| {
            let api = api_call.clone();
            let arguments: serde_json::Value =
                serde_json::from_str(&args_json).unwrap_or(serde_json::json!({}));

            let result =
                run_async_blocking(
                    async move { api.function_call(&function_path, arguments).await },
                );

            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "function_call failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("functions_call", call_fn)?;

    Ok(())
}

/// Register internal tasks API functions.
pub(super) fn register_tasks_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // task_create
    let api_create = api.clone();
    let create_fn = Function::new(ctx.clone(), move |request_json: String| {
        let api = api_create.clone();
        let request: serde_json::Value =
            serde_json::from_str(&request_json).unwrap_or(serde_json::json!({}));
        let result = run_async_blocking(async move { api.task_create(request).await });
        match result {
            Ok(v) => serde_json::to_string(&v)
                .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
            Err(e) => {
                tracing::error!(error = %e, "task_create failed");
                json_error(&e)
            }
        }
    })?;
    internal.set("task_create", create_fn)?;

    Ok(())
}

/// Register internal crypto API functions.
pub(super) fn register_crypto_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
) -> std::result::Result<(), rquickjs::Error> {
    let uuid_fn = Function::new(ctx.clone(), move || -> String {
        uuid::Uuid::new_v4().to_string()
    })?;
    internal.set("crypto_uuid", uuid_fn)?;

    Ok(())
}
