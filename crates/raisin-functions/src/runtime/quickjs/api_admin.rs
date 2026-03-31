// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Admin API registration for the QuickJS runtime.
//!
//! Registers internal functions for admin-level operations that bypass
//! RLS (Row Level Security) filtering. Access is controlled by the
//! `allows_admin_escalation` check.

use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;

use super::helpers::{json_error, json_error_with_fields, run_async_blocking};
use crate::api::FunctionApi;

/// Register internal admin API functions.
///
/// These functions provide admin-level access that bypasses RLS filtering.
/// Access is controlled by the `allows_admin_escalation` check.
pub(super) fn register_admin_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // allows_admin_escalation - checks if function has permission to escalate
    let api_check = api.clone();
    let check_fn = Function::new(ctx.clone(), move || api_check.allows_admin_escalation())?;
    internal.set("allows_admin_escalation", check_fn)?;

    register_admin_node_ops(ctx, internal, api.clone())?;
    register_admin_sql_ops(ctx, internal, api)?;

    Ok(())
}

/// Register admin node operations (get, getById, create, update, delete, query, getChildren, updateProperty).
fn register_admin_node_ops<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // admin_nodes_get
    let api_get = api.clone();
    let get_fn = Function::new(ctx.clone(), move |workspace: String, path: String| {
        let api = api_get.clone();
        let result = run_async_blocking(async move { api.admin_node_get(&workspace, &path).await });
        match result {
            Ok(Some(v)) => serde_json::to_string(&v).unwrap_or("null".to_string()),
            Ok(None) => "null".to_string(),
            Err(e) => {
                tracing::error!(error = %e, "admin_nodes_get failed");
                "null".to_string()
            }
        }
    })?;
    internal.set("admin_nodes_get", get_fn)?;

    // admin_nodes_getById
    let api_get_by_id = api.clone();
    let get_by_id_fn = Function::new(ctx.clone(), move |workspace: String, id: String| {
        let api = api_get_by_id.clone();
        let result =
            run_async_blocking(async move { api.admin_node_get_by_id(&workspace, &id).await });
        match result {
            Ok(Some(v)) => serde_json::to_string(&v).unwrap_or("null".to_string()),
            Ok(None) => "null".to_string(),
            Err(e) => {
                tracing::error!(error = %e, "admin_nodes_getById failed");
                "null".to_string()
            }
        }
    })?;
    internal.set("admin_nodes_getById", get_by_id_fn)?;

    // admin_nodes_create
    let api_create = api.clone();
    let create_fn = Function::new(
        ctx.clone(),
        move |workspace: String, parent: String, data_json: String| {
            let api = api_create.clone();
            let data: serde_json::Value =
                serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
            let result = run_async_blocking(async move {
                api.admin_node_create(&workspace, &parent, data).await
            });
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "admin_nodes_create failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("admin_nodes_create", create_fn)?;

    // admin_nodes_update
    let api_update = api.clone();
    let update_fn = Function::new(
        ctx.clone(),
        move |workspace: String, path: String, data_json: String| {
            let api = api_update.clone();
            let data: serde_json::Value =
                serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
            let result =
                run_async_blocking(
                    async move { api.admin_node_update(&workspace, &path, data).await },
                );
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "admin_nodes_update failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("admin_nodes_update", update_fn)?;

    // admin_nodes_delete
    let api_delete = api.clone();
    let delete_fn = Function::new(ctx.clone(), move |workspace: String, path: String| {
        let api = api_delete.clone();
        let result =
            run_async_blocking(async move { api.admin_node_delete(&workspace, &path).await });
        match result {
            Ok(()) => true,
            Err(e) => {
                tracing::error!(error = %e, "admin_nodes_delete failed");
                false
            }
        }
    })?;
    internal.set("admin_nodes_delete", delete_fn)?;

    // admin_nodes_query
    let api_query = api.clone();
    let query_fn = Function::new(ctx.clone(), move |workspace: String, query_json: String| {
        let api = api_query.clone();
        let query: serde_json::Value =
            serde_json::from_str(&query_json).unwrap_or(serde_json::json!({}));
        let result =
            run_async_blocking(async move { api.admin_node_query(&workspace, query).await });
        match result {
            Ok(v) => serde_json::to_string(&v).unwrap_or("[]".to_string()),
            Err(e) => {
                tracing::error!(error = %e, "admin_nodes_query failed");
                "[]".to_string()
            }
        }
    })?;
    internal.set("admin_nodes_query", query_fn)?;

    // admin_nodes_getChildren
    let api_children = api.clone();
    let children_fn = Function::new(
        ctx.clone(),
        move |workspace: String, path: String, limit: Option<u32>| {
            let api = api_children.clone();
            let result = run_async_blocking(async move {
                api.admin_node_get_children(&workspace, &path, limit).await
            });
            match result {
                Ok(v) => serde_json::to_string(&v).unwrap_or("[]".to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "admin_nodes_getChildren failed");
                    "[]".to_string()
                }
            }
        },
    )?;
    internal.set("admin_nodes_getChildren", children_fn)?;

    // admin_nodes_updateProperty
    let api_update_prop = api.clone();
    let update_prop_fn = Function::new(
        ctx.clone(),
        move |workspace: String, node_path: String, property_path: String, value_json: String| {
            let api = api_update_prop.clone();
            let value: serde_json::Value =
                serde_json::from_str(&value_json).unwrap_or(serde_json::Value::Null);
            let result = run_async_blocking(async move {
                api.admin_node_update_property(&workspace, &node_path, &property_path, value)
                    .await
            });
            match result {
                Ok(()) => true,
                Err(e) => {
                    tracing::error!(error = %e, "admin_nodes_updateProperty failed");
                    false
                }
            }
        },
    )?;
    internal.set("admin_nodes_updateProperty", update_prop_fn)?;

    Ok(())
}

/// Register admin SQL operations (query, execute).
fn register_admin_sql_ops<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // admin_sql_query
    let api_sql_query = api.clone();
    let sql_query_fn = Function::new(
        ctx.clone(),
        move |sql_str: String, params_json: Option<String>| {
            let api = api_sql_query.clone();
            let params_vec: Vec<serde_json::Value> = params_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let result =
                run_async_blocking(async move { api.admin_sql_query(&sql_str, params_vec).await });
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed","rows":[]}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "admin_sql_query failed");
                    json_error_with_fields(&e, serde_json::json!({"rows": []}))
                }
            }
        },
    )?;
    internal.set("admin_sql_query", sql_query_fn)?;

    // admin_sql_execute
    let api_sql_execute = api.clone();
    let sql_execute_fn = Function::new(
        ctx.clone(),
        move |sql_str: String, params_json: Option<String>| {
            let api = api_sql_execute.clone();
            let params_vec: Vec<serde_json::Value> = params_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let result =
                run_async_blocking(
                    async move { api.admin_sql_execute(&sql_str, params_vec).await },
                );
            match result {
                Ok(count) => count,
                Err(e) => {
                    tracing::error!(error = %e, "admin_sql_execute failed");
                    -1
                }
            }
        },
    )?;
    internal.set("admin_sql_execute", sql_execute_fn)?;

    Ok(())
}
