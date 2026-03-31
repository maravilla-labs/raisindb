// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Admin operation API bindings (bypass RLS)

use crate::api::FunctionApi;
use crate::runtime::bindings::registry::{
    ApiMethodDescriptor, ArgParser, ArgSpec, ArgType, InvokeResult, ReturnType,
};
use futures::future::BoxFuture;
use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Get all admin operation method descriptors
///
/// These methods are exposed under `raisin.admin.*` and bypass RLS filtering.
/// Only available to functions with `requiresAdmin: true` in metadata.
pub fn methods() -> Vec<ApiMethodDescriptor> {
    vec![
        // admin.nodes.get(workspace, path)
        ApiMethodDescriptor {
            internal_name: "admin_nodes_get",
            js_name: "get",
            py_name: "get",
            category: "admin_nodes",
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
                    let result = api.admin_node_get(&workspace, &path).await?;
                    Ok(InvokeResult::OptionalJson(result))
                })
            },
        },
        // admin.nodes.getById(workspace, id)
        ApiMethodDescriptor {
            internal_name: "admin_nodes_getById",
            js_name: "getById",
            py_name: "get_by_id",
            category: "admin_nodes",
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
                    let result = api.admin_node_get_by_id(&workspace, &id).await?;
                    Ok(InvokeResult::OptionalJson(result))
                })
            },
        },
        // admin.nodes.create(workspace, parentPath, data)
        ApiMethodDescriptor {
            internal_name: "admin_nodes_create",
            js_name: "create",
            py_name: "create",
            category: "admin_nodes",
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
                    let result = api
                        .admin_node_create(&workspace, &parent_path, data)
                        .await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // admin.nodes.update(workspace, path, data)
        ApiMethodDescriptor {
            internal_name: "admin_nodes_update",
            js_name: "update",
            py_name: "update",
            category: "admin_nodes",
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
                    let result = api.admin_node_update(&workspace, &path, data).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // admin.nodes.delete(workspace, path)
        ApiMethodDescriptor {
            internal_name: "admin_nodes_delete",
            js_name: "delete",
            py_name: "delete",
            category: "admin_nodes",
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
                    api.admin_node_delete(&workspace, &path).await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // admin.nodes.updateProperty(workspace, nodePath, propertyPath, value)
        ApiMethodDescriptor {
            internal_name: "admin_nodes_updateProperty",
            js_name: "updateProperty",
            py_name: "update_property",
            category: "admin_nodes",
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
                    api.admin_node_update_property(&workspace, &node_path, &property_path, value)
                        .await?;
                    Ok(InvokeResult::Void)
                })
            },
        },
        // admin.nodes.query(workspace, query)
        ApiMethodDescriptor {
            internal_name: "admin_nodes_query",
            js_name: "query",
            py_name: "query",
            category: "admin_nodes",
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
                    let result = api.admin_node_query(&workspace, query).await?;
                    Ok(InvokeResult::JsonArray(result))
                })
            },
        },
        // admin.nodes.getChildren(workspace, parentPath, limit?)
        ApiMethodDescriptor {
            internal_name: "admin_nodes_getChildren",
            js_name: "getChildren",
            py_name: "get_children",
            category: "admin_nodes",
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
                        .admin_node_get_children(&workspace, &parent_path, limit)
                        .await?;
                    Ok(InvokeResult::JsonArray(result))
                })
            },
        },
        // admin.sql.query(sql, params)
        ApiMethodDescriptor {
            internal_name: "admin_sql_query",
            js_name: "query",
            py_name: "query",
            category: "admin_sql",
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
                    let result = api.admin_sql_query(&sql, params).await?;
                    Ok(InvokeResult::Json(result))
                })
            },
        },
        // admin.sql.execute(sql, params)
        ApiMethodDescriptor {
            internal_name: "admin_sql_execute",
            js_name: "execute",
            py_name: "execute",
            category: "admin_sql",
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
                    let result = api.admin_sql_execute(&sql, params).await?;
                    Ok(InvokeResult::I64(result))
                })
            },
        },
    ]
}
