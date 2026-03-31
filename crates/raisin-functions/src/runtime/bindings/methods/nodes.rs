// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Node operation API bindings

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all node operation method descriptors
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // nodes.get(workspace, path)
        ApiMethodDescriptor {
            internal_name: "nodes_get",
            js_name: "get",
            py_name: "get",
            category: "nodes",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("path", ArgType::String),
            ],
            return_type: ReturnType::OptionalJson,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let path = parser.string()?;
                    let result = api.node_get(&workspace, &path).await?;
                    Ok(InvokeResult::OptionalJson(result))
                })
            },
        },
        // nodes.getById(workspace, id)
        ApiMethodDescriptor {
            internal_name: "nodes_getById",
            js_name: "getById",
            py_name: "get_by_id",
            category: "nodes",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("id", ArgType::String),
            ],
            return_type: ReturnType::OptionalJson,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let id = parser.string()?;
                    let result = api.node_get_by_id(&workspace, &id).await?;
                    Ok(InvokeResult::OptionalJson(result))
                })
            },
        },
        // nodes.create(workspace, parentPath, data)
        ApiMethodDescriptor {
            internal_name: "nodes_create",
            js_name: "create",
            py_name: "create",
            category: "nodes",
            args: vec![
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
                    let workspace = parser.string()?;
                    let parent_path = parser.string()?;
                    let data = parser.json()?;
                    let result = api.node_create(&workspace, &parent_path, data).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // nodes.update(workspace, path, data)
        ApiMethodDescriptor {
            internal_name: "nodes_update",
            js_name: "update",
            py_name: "update",
            category: "nodes",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("path", ArgType::String),
                ArgSpec::new("data", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let path = parser.string()?;
                    let data = parser.json()?;
                    let result = api.node_update(&workspace, &path, data).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // nodes.delete(workspace, path)
        ApiMethodDescriptor {
            internal_name: "nodes_delete",
            js_name: "delete",
            py_name: "delete",
            category: "nodes",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("path", ArgType::String),
            ],
            return_type: ReturnType::Void,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let path = parser.string()?;
                    api.node_delete(&workspace, &path).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // nodes.updateProperty(workspace, nodePath, propertyPath, value)
        ApiMethodDescriptor {
            internal_name: "nodes_updateProperty",
            js_name: "updateProperty",
            py_name: "update_property",
            category: "nodes",
            args: vec![
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
                    let workspace = parser.string()?;
                    let node_path = parser.string()?;
                    let property_path = parser.string()?;
                    let value = parser.json()?;
                    api.node_update_property(&workspace, &node_path, &property_path, value)
                        .await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // nodes.move(workspace, nodePath, newParentPath)
        ApiMethodDescriptor {
            internal_name: "nodes_move",
            js_name: "move",
            py_name: "move",
            category: "nodes",
            args: vec![
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
                    let workspace = parser.string()?;
                    let node_path = parser.string()?;
                    let new_parent_path = parser.string()?;
                    let result = api
                        .node_move(&workspace, &node_path, &new_parent_path)
                        .await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // nodes.query(workspace, query)
        ApiMethodDescriptor {
            internal_name: "nodes_query",
            js_name: "query",
            py_name: "query",
            category: "nodes",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("query", ArgType::Json),
            ],
            return_type: ReturnType::JsonArray,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let query = parser.json()?;
                    let result = api.node_query(&workspace, query).await?;
                    Ok(InvokeResult::JsonArray(result))
                })
            },
        },
        // nodes.getChildren(workspace, parentPath, limit?)
        ApiMethodDescriptor {
            internal_name: "nodes_getChildren",
            js_name: "getChildren",
            py_name: "get_children",
            category: "nodes",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("parentPath", ArgType::String),
                ArgSpec::new("limit", ArgType::OptionalU32),
            ],
            return_type: ReturnType::JsonArray,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let parent_path = parser.string()?;
                    let limit = parser.optional_u32()?;
                    let result = api
                        .node_get_children(&workspace, &parent_path, limit)
                        .await?;
                    Ok(InvokeResult::JsonArray(result))
                })
            },
        },
        // nodes.addResource(workspace, nodePath, propertyPath, uploadData)
        ApiMethodDescriptor {
            internal_name: "nodes_addResource",
            js_name: "addResource",
            py_name: "add_resource",
            category: "nodes",
            args: vec![
                ArgSpec::new("workspace", ArgType::String),
                ArgSpec::new("nodePath", ArgType::String),
                ArgSpec::new("propertyPath", ArgType::String),
                ArgSpec::new("uploadData", ArgType::Json),
            ],
            return_type: ReturnType::Json,
            invoker: |api: Arc<dyn FunctionApi>,
                      args: Vec<Value>|
             -> BoxFuture<'static, Result<InvokeResult>> {
                Box::pin(async move {
                    let mut parser = ArgParser::new(&args);
                    let workspace = parser.string()?;
                    let node_path = parser.string()?;
                    let property_path = parser.string()?;
                    let upload_data = parser.json()?;
                    let result = api
                        .node_add_resource(&workspace, &node_path, &property_path, upload_data)
                        .await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
    ]
}
