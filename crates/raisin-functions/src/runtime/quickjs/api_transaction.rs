// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Transaction API registration for the QuickJS runtime.
//!
//! Registers internal functions for transaction operations:
//! begin, commit, rollback, create, add, put, upsert, update, delete,
//! get, get_by_path, list_children, update_property, set_actor, set_message,
//! create_deep, upsert_deep.

use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;

use super::helpers::{json_error, run_async_blocking};
use crate::api::FunctionApi;

/// Register internal transaction API functions.
pub(super) fn register_transaction_internal<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    register_tx_lifecycle(ctx, internal, api.clone())?;
    register_tx_crud(ctx, internal, api.clone())?;
    register_tx_read(ctx, internal, api.clone())?;
    register_tx_property(ctx, internal, api)?;

    Ok(())
}

/// Register transaction lifecycle operations (begin, commit, rollback, set_actor, set_message).
fn register_tx_lifecycle<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // tx_begin
    let api_begin = api.clone();
    let begin_fn = Function::new(ctx.clone(), move || {
        let api = api_begin.clone();
        let result = run_async_blocking(async move { api.tx_begin().await });
        match result {
            Ok(tx_id) => tx_id,
            Err(e) => {
                tracing::error!(error = %e, "tx_begin failed");
                format!("error:{}", e)
            }
        }
    })?;
    internal.set("tx_begin", begin_fn)?;

    // tx_commit
    let api_commit = api.clone();
    let commit_fn = Function::new(ctx.clone(), move |tx_id: String| {
        let api = api_commit.clone();
        let result = run_async_blocking(async move { api.tx_commit(&tx_id).await });
        match result {
            Ok(()) => true,
            Err(e) => {
                tracing::error!(error = %e, "tx_commit failed");
                false
            }
        }
    })?;
    internal.set("tx_commit", commit_fn)?;

    // tx_rollback
    let api_rollback = api.clone();
    let rollback_fn = Function::new(ctx.clone(), move |tx_id: String| {
        let api = api_rollback.clone();
        let result = run_async_blocking(async move { api.tx_rollback(&tx_id).await });
        match result {
            Ok(()) => true,
            Err(e) => {
                tracing::error!(error = %e, "tx_rollback failed");
                false
            }
        }
    })?;
    internal.set("tx_rollback", rollback_fn)?;

    // tx_set_actor
    let api_set_actor = api.clone();
    let set_actor_fn = Function::new(ctx.clone(), move |tx_id: String, actor: String| {
        let api = api_set_actor.clone();
        let result = run_async_blocking(async move { api.tx_set_actor(&tx_id, &actor).await });
        match result {
            Ok(()) => true,
            Err(e) => {
                tracing::error!(error = %e, "tx_set_actor failed");
                false
            }
        }
    })?;
    internal.set("tx_set_actor", set_actor_fn)?;

    // tx_set_message
    let api_set_message = api.clone();
    let set_message_fn = Function::new(ctx.clone(), move |tx_id: String, message: String| {
        let api = api_set_message.clone();
        let result = run_async_blocking(async move { api.tx_set_message(&tx_id, &message).await });
        match result {
            Ok(()) => true,
            Err(e) => {
                tracing::error!(error = %e, "tx_set_message failed");
                false
            }
        }
    })?;
    internal.set("tx_set_message", set_message_fn)?;

    Ok(())
}

/// Register transaction CRUD operations (create, add, put, upsert, update, delete, create_deep, upsert_deep).
fn register_tx_crud<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // tx_create
    let api_create = api.clone();
    let create_fn = Function::new(
        ctx.clone(),
        move |tx_id: String, workspace: String, parent_path: String, data_json: String| {
            let api = api_create.clone();
            let data: serde_json::Value =
                serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
            let result = run_async_blocking(async move {
                api.tx_create(&tx_id, &workspace, &parent_path, data).await
            });
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "tx_create failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("tx_create", create_fn)?;

    // tx_add
    let api_add = api.clone();
    let add_fn = Function::new(
        ctx.clone(),
        move |tx_id: String, workspace: String, data_json: String| {
            let api = api_add.clone();
            let data: serde_json::Value =
                serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
            let result =
                run_async_blocking(async move { api.tx_add(&tx_id, &workspace, data).await });
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "tx_add failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("tx_add", add_fn)?;

    // tx_put
    let api_put = api.clone();
    let put_fn = Function::new(
        ctx.clone(),
        move |tx_id: String, workspace: String, data_json: String| {
            let api = api_put.clone();
            let data: serde_json::Value =
                serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
            let result =
                run_async_blocking(async move { api.tx_put(&tx_id, &workspace, data).await });
            match result {
                Ok(()) => "true".to_string(),
                Err(e) => {
                    tracing::error!(error = %e, "tx_put failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("tx_put", put_fn)?;

    // tx_upsert
    let api_upsert = api.clone();
    let upsert_fn = Function::new(
        ctx.clone(),
        move |tx_id: String, workspace: String, data_json: String| {
            let api = api_upsert.clone();
            let data: serde_json::Value =
                serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
            let result =
                run_async_blocking(async move { api.tx_upsert(&tx_id, &workspace, data).await });
            match result {
                Ok(()) => "true".to_string(),
                Err(e) => {
                    tracing::error!(error = %e, "tx_upsert failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("tx_upsert", upsert_fn)?;

    // tx_create_deep
    let api_create_deep = api.clone();
    let create_deep_fn = Function::new(
        ctx.clone(),
        move |tx_id: String,
              workspace: String,
              parent_path: String,
              data_json: String,
              parent_node_type: String| {
            let api = api_create_deep.clone();
            let data: serde_json::Value =
                serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
            let result = run_async_blocking(async move {
                api.tx_create_deep(&tx_id, &workspace, &parent_path, data, &parent_node_type)
                    .await
            });
            match result {
                Ok(v) => serde_json::to_string(&v)
                    .unwrap_or(r#"{"error":"serialization failed"}"#.to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "tx_create_deep failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("tx_create_deep", create_deep_fn)?;

    // tx_upsert_deep
    let api_upsert_deep = api.clone();
    let upsert_deep_fn = Function::new(
        ctx.clone(),
        move |tx_id: String, workspace: String, data_json: String, parent_node_type: String| {
            let api = api_upsert_deep.clone();
            let data: serde_json::Value =
                serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
            let result = run_async_blocking(async move {
                api.tx_upsert_deep(&tx_id, &workspace, data, &parent_node_type)
                    .await
            });
            match result {
                Ok(()) => "true".to_string(),
                Err(e) => {
                    tracing::error!(error = %e, "tx_upsert_deep failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("tx_upsert_deep", upsert_deep_fn)?;

    // tx_update
    let api_update = api.clone();
    let update_fn = Function::new(
        ctx.clone(),
        move |tx_id: String, workspace: String, path: String, data_json: String| {
            let api = api_update.clone();
            let data: serde_json::Value =
                serde_json::from_str(&data_json).unwrap_or(serde_json::json!({}));
            let result =
                run_async_blocking(
                    async move { api.tx_update(&tx_id, &workspace, &path, data).await },
                );
            match result {
                Ok(()) => "true".to_string(),
                Err(e) => {
                    tracing::error!(error = %e, "tx_update failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("tx_update", update_fn)?;

    // tx_delete
    let api_delete = api.clone();
    let delete_fn = Function::new(
        ctx.clone(),
        move |tx_id: String, workspace: String, path: String| {
            let api = api_delete.clone();
            let result =
                run_async_blocking(async move { api.tx_delete(&tx_id, &workspace, &path).await });
            match result {
                Ok(()) => true,
                Err(e) => {
                    tracing::error!(error = %e, "tx_delete failed");
                    false
                }
            }
        },
    )?;
    internal.set("tx_delete", delete_fn)?;

    // tx_delete_by_id
    let api_delete_by_id = api.clone();
    let delete_by_id_fn = Function::new(
        ctx.clone(),
        move |tx_id: String, workspace: String, id: String| {
            let api = api_delete_by_id.clone();
            let result =
                run_async_blocking(
                    async move { api.tx_delete_by_id(&tx_id, &workspace, &id).await },
                );
            match result {
                Ok(()) => true,
                Err(e) => {
                    tracing::error!(error = %e, "tx_delete_by_id failed");
                    false
                }
            }
        },
    )?;
    internal.set("tx_delete_by_id", delete_by_id_fn)?;

    Ok(())
}

/// Register transaction read operations (get, get_by_path, list_children).
fn register_tx_read<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // tx_get
    let api_get = api.clone();
    let get_fn = Function::new(
        ctx.clone(),
        move |tx_id: String, workspace: String, id: String| {
            let api = api_get.clone();
            let result =
                run_async_blocking(async move { api.tx_get(&tx_id, &workspace, &id).await });
            match result {
                Ok(Some(v)) => serde_json::to_string(&v).unwrap_or("null".to_string()),
                Ok(None) => "null".to_string(),
                Err(e) => {
                    tracing::error!(error = %e, "tx_get failed");
                    "null".to_string()
                }
            }
        },
    )?;
    internal.set("tx_get", get_fn)?;

    // tx_get_by_path
    let api_get_by_path = api.clone();
    let get_by_path_fn = Function::new(
        ctx.clone(),
        move |tx_id: String, workspace: String, path: String| {
            let api = api_get_by_path.clone();
            let result =
                run_async_blocking(
                    async move { api.tx_get_by_path(&tx_id, &workspace, &path).await },
                );
            match result {
                Ok(Some(v)) => serde_json::to_string(&v).unwrap_or("null".to_string()),
                Ok(None) => "null".to_string(),
                Err(e) => {
                    tracing::error!(error = %e, "tx_get_by_path failed");
                    "null".to_string()
                }
            }
        },
    )?;
    internal.set("tx_get_by_path", get_by_path_fn)?;

    // tx_list_children
    let api_list_children = api.clone();
    let list_children_fn = Function::new(
        ctx.clone(),
        move |tx_id: String, workspace: String, parent_path: String| {
            let api = api_list_children.clone();
            let result = run_async_blocking(async move {
                api.tx_list_children(&tx_id, &workspace, &parent_path).await
            });
            match result {
                Ok(v) => serde_json::to_string(&v).unwrap_or("[]".to_string()),
                Err(e) => {
                    tracing::error!(error = %e, "tx_list_children failed");
                    "[]".to_string()
                }
            }
        },
    )?;
    internal.set("tx_list_children", list_children_fn)?;

    // NOTE: tx_move is intentionally NOT exposed.
    // Move requires target parent to be committed, which conflicts with transaction semantics.
    // For "move" within a transaction, use: tx.delete(oldPath) + tx.add(newPath, { id: sameId, ... })

    Ok(())
}

/// Register transaction property update operation.
fn register_tx_property<'js>(
    ctx: &Ctx<'js>,
    internal: &Object<'js>,
    api: Arc<dyn FunctionApi>,
) -> std::result::Result<(), rquickjs::Error> {
    // tx_update_property
    let api_update_property = api.clone();
    let update_property_fn = Function::new(
        ctx.clone(),
        move |tx_id: String,
              workspace: String,
              node_path: String,
              property_path: String,
              value: String| {
            let api = api_update_property.clone();
            let parsed_value: serde_json::Value =
                serde_json::from_str(&value).unwrap_or(serde_json::Value::Null);
            let result = run_async_blocking(async move {
                api.tx_update_property(&tx_id, &workspace, &node_path, &property_path, parsed_value)
                    .await
            });
            match result {
                Ok(()) => "true".to_string(),
                Err(e) => {
                    tracing::error!(error = %e, "tx_update_property failed");
                    json_error(&e)
                }
            }
        },
    )?;
    internal.set("tx_update_property", update_property_fn)?;

    Ok(())
}
