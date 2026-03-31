// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Transaction operation API bindings

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all transaction operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // tx.begin()
        ApiMethodDescriptor {
            internal_name: "tx_begin",
            js_name: "begin",
            py_name: "begin",
            category: "tx",
            args: vec![],
            return_type: ReturnType::String,
            invoker: |api: Arc<dyn FunctionApi>,
                      _args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let tx_id = api.tx_begin().await?;
                    Ok(InvokeResult::String(tx_id))
                })
            },
        },
        // tx.commit(txId)
        ApiMethodDescriptor {
            internal_name: "tx_commit",
            js_name: "commit",
            py_name: "commit",
            category: "tx",
            args: vec![ArgSpec::new("txId", ArgType::String)],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    api.tx_commit(&tx_id).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // tx.rollback(txId)
        ApiMethodDescriptor {
            internal_name: "tx_rollback",
            js_name: "rollback",
            py_name: "rollback",
            category: "tx",
            args: vec![ArgSpec::new("txId", ArgType::String)],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    api.tx_rollback(&tx_id).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // tx.setActor(txId, actor)
        ApiMethodDescriptor {
            internal_name: "tx_setActor",
            js_name: "setActor",
            py_name: "set_actor",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("actor", ArgType::String),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let actor = parser.string()?;
                    api.tx_set_actor(&tx_id, &actor).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // tx.setMessage(txId, message)
        ApiMethodDescriptor {
            internal_name: "tx_setMessage",
            js_name: "setMessage",
            py_name: "set_message",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("message", ArgType::String),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let message = parser.string()?;
                    api.tx_set_message(&tx_id, &message).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // tx.create(txId, workspace, parentPath, data)
        ApiMethodDescriptor {
            internal_name: "tx_create",
            js_name: "create",
            py_name: "create",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("parentPath", ArgType::String),
                ArgSpec::new("data", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let parent_path = parser.string()?;
                    let data = parser.json()?;
                    let result = api
                        .tx_create(&tx_id, &workspace, &parent_path, data)
                        .await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // tx.add(txId, workspace, data)
        ApiMethodDescriptor {
            internal_name: "tx_add",
            js_name: "add",
            py_name: "add",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("data", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let data = parser.json()?;
                    let result = api.tx_add(&tx_id, &workspace, data).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // tx.put(txId, workspace, data)
        ApiMethodDescriptor {
            internal_name: "tx_put",
            js_name: "put",
            py_name: "put",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("data", ArgType::Json),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let data = parser.json()?;
                    api.tx_put(&tx_id, &workspace, data).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // tx.upsert(txId, workspace, data)
        ApiMethodDescriptor {
            internal_name: "tx_upsert",
            js_name: "upsert",
            py_name: "upsert",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("data", ArgType::Json),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let data = parser.json()?;
                    api.tx_upsert(&tx_id, &workspace, data).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // tx.createDeep(txId, workspace, parentPath, data, parentNodeType)
        ApiMethodDescriptor {
            internal_name: "tx_createDeep",
            js_name: "createDeep",
            py_name: "create_deep",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("parentPath", ArgType::String),
                ArgSpec::new("data", ArgType::Json),
                ArgSpec::new("parentNodeType", ArgType::String),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let parent_path = parser.string()?;
                    let data = parser.json()?;
                    let parent_node_type = parser.string()?;
                    let result = api
                        .tx_create_deep(&tx_id, &workspace, &parent_path, data, &parent_node_type)
                        .await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // tx.upsertDeep(txId, workspace, data, parentNodeType)
        ApiMethodDescriptor {
            internal_name: "tx_upsertDeep",
            js_name: "upsertDeep",
            py_name: "upsert_deep",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("data", ArgType::Json),
                ArgSpec::new("parentNodeType", ArgType::String),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let data = parser.json()?;
                    let parent_node_type = parser.string()?;
                    api.tx_upsert_deep(&tx_id, &workspace, data, &parent_node_type)
                        .await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // tx.update(txId, workspace, path, data)
        ApiMethodDescriptor {
            internal_name: "tx_update",
            js_name: "update",
            py_name: "update",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("path", ArgType::String),
                ArgSpec::new("data", ArgType::Json),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let path = parser.string()?;
                    let data = parser.json()?;
                    api.tx_update(&tx_id, &workspace, &path, data).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // tx.delete(txId, workspace, path)
        ApiMethodDescriptor {
            internal_name: "tx_delete",
            js_name: "delete",
            py_name: "delete",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("path", ArgType::String),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let path = parser.string()?;
                    api.tx_delete(&tx_id, &workspace, &path).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // tx.deleteById(txId, workspace, id)
        ApiMethodDescriptor {
            internal_name: "tx_deleteById",
            js_name: "deleteById",
            py_name: "delete_by_id",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("id", ArgType::String),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let id = parser.string()?;
                    api.tx_delete_by_id(&tx_id, &workspace, &id).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // tx.get(txId, workspace, id)
        ApiMethodDescriptor {
            internal_name: "tx_get",
            js_name: "get",
            py_name: "get",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("id", ArgType::String),
            ],
            return_type: ReturnType::OptionalJson,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let id = parser.string()?;
                    let result = api.tx_get(&tx_id, &workspace, &id).await?;
                    Ok(InvokeResult::OptionalJson(result))
                })
            },
        },
        // tx.getByPath(txId, workspace, path)
        ApiMethodDescriptor {
            internal_name: "tx_getByPath",
            js_name: "getByPath",
            py_name: "get_by_path",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("path", ArgType::String),
            ],
            return_type: ReturnType::OptionalJson,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let path = parser.string()?;
                    let result = api.tx_get_by_path(&tx_id, &workspace, &path).await?;
                    Ok(InvokeResult::OptionalJson(result))
                })
            },
        },
        // tx.listChildren(txId, workspace, parentPath)
        ApiMethodDescriptor {
            internal_name: "tx_listChildren",
            js_name: "listChildren",
            py_name: "list_children",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("parentPath", ArgType::String),
            ],
            return_type: ReturnType::JsonArray,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let parent_path = parser.string()?;
                    let result = api
                        .tx_list_children(&tx_id, &workspace, &parent_path)
                        .await?;
                    Ok(InvokeResult::JsonArray(result))
                })
            },
        },
        // tx.move(txId, workspace, nodePath, newParentPath)
        ApiMethodDescriptor {
            internal_name: "tx_move",
            js_name: "move",
            py_name: "move",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("nodePath", ArgType::String),
                ArgSpec::new("newParentPath", ArgType::String),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let node_path = parser.string()?;
                    let new_parent_path = parser.string()?;
                    let result = api
                        .tx_move(&tx_id, &workspace, &node_path, &new_parent_path)
                        .await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // tx.updateProperty(txId, workspace, nodePath, propertyPath, value)
        ApiMethodDescriptor {
            internal_name: "tx_updateProperty",
            js_name: "updateProperty",
            py_name: "update_property",
            category: "tx",
            args: vec![
                ArgSpec::new("txId", ArgType::String),
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("nodePath", ArgType::String),
                ArgSpec::new("propertyPath", ArgType::String),
                ArgSpec::new("value", ArgType::Json),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let tx_id = parser.string()?;
                    let workspace = parser.string()?;
                    let node_path = parser.string()?;
                    let property_path = parser.string()?;
                    let value = parser.json()?;
                    api.tx_update_property(&tx_id, &workspace, &node_path, &property_path, value)
                        .await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
    ]
}
