// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Node API registration for the QuickJS runtime.
//!
//! Registers internal functions for CRUD operations on nodes:
//! get, getById, create, update, delete, query, getChildren, updateProperty, move.

use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;

use super::helpers::{json_error, run_async_blocking};
use crate::api::FunctionApi;

/// Register internal nodes API functions.
pub(super) fn register_nodes_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // nodes_get
    let api_get = api.clone();
    let get_fn = Function::new(ctx.clone(), move |workspace: String, path: String| {
        let api = api_get.clone();
        let result = run_async_blocking(async move { api.node_get(&workspace, &path).await });
        match result {
            Ok(Some(v)) => serde_json::to_string(&v).unwrap_or("null".to_string()),
            Ok(None) => "null".to_string(),
            Err(e) => {
                tracing::error!(error = %e, "node_get failed");
                "null".to_string()
            }
        }
    })?;
    internal.set("nodes_get", get_fn)?;

    // nodes_getById
    let api_get_by_id = api.clone();
    let get_by_id_fn = Function::new(ctx.clone(), move |workspace: String, id: String| {
        let api = api_get_by_id.clone();
        let result = run_async_blocking(async move { api.node_get_by_id(&workspace, &id).await });
        match result {
            Ok(Some(v)) => serde_json::to_string(&v).unwrap_or("null".to_string()),
            Ok(None) => "null".to_string(),
            Err(e) => {
                tracing::error!(error = %e, "node_get_by_id failed");
                "null".to_string()
            }
        }
    })?;
    internal.set("nodes_getById", get_by_id_fn)?;

    // nodes_create
    let api_create = api.clone();
    let create_fn = Function::new(
        ctx.clone(),
        move |workspace: String, parent: String, data_json: String| {
            let api = api_create.clone();
            let data: serde_json::Value =
                serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
            let result =
                run_async_blocking(async move { api.node_create(&workspace, &parent, data).await });
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "node_create failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("nodes_create", create_fn)?;

    // nodes_update
    let api_update = api.clone();
    let update_fn = Function::new(
        ctx.clone(),
        move |workspace: String, path: String, data_json: String| {
            let api = api_update.clone();
            let data: serde_json::Value =
                serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
            let result =
                run_async_blocking(async move { api.node_update(&workspace, &path, data).await });
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "node_update failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("nodes_update", update_fn)?;

    // nodes_delete
    let api_delete = api.clone();
    let delete_fn = Function::new(ctx.clone(), move |workspace: String, path: String| {
        let api = api_delete.clone();
        let result = run_async_blocking(async move { api.node_delete(&workspace, &path).await });
        match result {
            Ok(()) => true,
            Err(e) => {
                tracing::error!(error = %e, "node_delete failed");
                false
            }
        }
    })?;
    internal.set("nodes_delete", delete_fn)?;

    // nodes_query
    let api_query = api.clone();
    let query_fn = Function::new(ctx.clone(), move |workspace: String, query_json: String| {
        let api = api_query.clone();
        let query: serde_json::Value =
            serde_json::from_str(&query_json).unwrap_or(serde_json::json!({}));
        let result = run_async_blocking(async move { api.node_query(&workspace, query).await });
        match result {
            Ok(v) => serde_json::to_string(&v).unwrap_or("[]".to_string()),
            Err(e) => {
                tracing::error!(error = %e, "node_query failed");
                "[]".to_string()
            }
        }
    })?;
    internal.set("nodes_query", query_fn)?;

    // nodes_getChildren
    let api_children = api.clone();
    let children_fn = Function::new(
        ctx.clone(),
        move |workspace: String, path: String, limit: Option<u32>| {
            let api = api_children.clone();
            let result =
                run_async_blocking(
                    async move { api.node_get_children(&workspace, &path, limit).await },
                );
            match result {
                Ok(v) => serde_json::to_string(&v).unwrap_or("[]".to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "node_get_children failed");
                    "[]".to_string()
                }
            }
        },
    )?;
    internal.set("nodes_getChildren", children_fn)?;

    // nodes_updateProperty
    let api_update_prop = api.clone();
    let update_prop_fn = Function::new(
        ctx.clone(),
        move |workspace: String, node_path: String, property_path: String, value_json: String| {
            let api = api_update_prop.clone();
            let value: serde_json::Value =
                serde_json::from_str(&value_json).unwrap_or(serde_json::Value::Null);
            let result = run_async_blocking(async move {
                api.node_update_property(&workspace, &node_path, &property_path, value)
                    .await
            });
            match result {
                Ok(()) => true,
                Err(e) => {
                    tracing::error!(error = %e, "node_update_property failed");
                    false
                }
            }
        },
    )?;
    internal.set("nodes_updateProperty", update_prop_fn)?;

    // nodes_move
    let api_move = api.clone();
    let move_fn = Function::new(
        ctx.clone(),
        move |workspace: String, node_path: String, new_parent_path: String| {
            let api = api_move.clone();
            let result = run_async_blocking(async move {
                api.node_move(&workspace, &node_path, &new_parent_path)
                    .await
            });
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "node_move failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("nodes_move", move_fn)?;

    Ok(())
}
