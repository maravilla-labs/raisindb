// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Node operation callbacks for function execution.
//!
//! These callbacks implement the `raisin.nodes.*` API available to JavaScript functions.
//! All operations are routed through SQL execution to ensure consistent transaction handling
//! with auto-commit behavior matching `raisin-sql-execution`.

use std::sync::Arc;

use raisin_binary::BinaryStorage;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::RaisinReference;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use raisin_storage::StorageScope;
use serde_json::Value;

use super::query_context::QueryContext;
use super::sql_generator;
use crate::api::{
    NodeCreateCallback, NodeDeleteCallback, NodeGetByIdCallback, NodeGetCallback,
    NodeGetChildrenCallback, NodeMoveCallback, NodeQueryCallback, NodeUpdateCallback,
    NodeUpdatePropertyCallback,
};

// ============================================================================
// READ OPERATIONS
// ============================================================================

/// Create node_get callback: `raisin.nodes.get(workspace, path)`
///
/// Uses SQL SELECT to retrieve node, ensuring consistent RLS and transaction handling.
pub fn create_node_get<S, B>(query_ctx: Arc<QueryContext<S, B>>) -> NodeGetCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |workspace: String, path: String| {
        let ctx = query_ctx.clone();

        Box::pin(async move {
            let stmt = sql_generator::generate_select_by_path(&workspace, &path);
            let rows = ctx.execute_query(&stmt).await?;

            // Return first row or None
            Ok(rows.into_iter().next())
        })
    })
}

/// Create node_get_by_id callback: `raisin.nodes.getById(workspace, id)`
///
/// Uses SQL SELECT to retrieve node by ID.
pub fn create_node_get_by_id<S, B>(query_ctx: Arc<QueryContext<S, B>>) -> NodeGetByIdCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |workspace: String, id: String| {
        let ctx = query_ctx.clone();

        Box::pin(async move {
            let stmt = sql_generator::generate_select_by_id(&workspace, &id);
            let rows = ctx.execute_query(&stmt).await?;

            // Return first row or None
            Ok(rows.into_iter().next())
        })
    })
}

/// Create node_get_children callback: `raisin.nodes.getChildren(workspace, parentPath, limit)`
///
/// Uses SQL SELECT to retrieve child nodes.
pub fn create_node_get_children<S, B>(query_ctx: Arc<QueryContext<S, B>>) -> NodeGetChildrenCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(
        move |workspace: String, parent_path: String, limit: Option<u32>| {
            let ctx = query_ctx.clone();

            Box::pin(async move {
                let stmt = sql_generator::generate_select_children(&workspace, &parent_path, limit);
                ctx.execute_query(&stmt).await
            })
        },
    )
}

/// Create node_query callback: `raisin.nodes.query(workspace, query)`
///
/// NOTE: Stub - use SQL for complex queries.
pub fn create_node_query<S, B>(_query_ctx: Arc<QueryContext<S, B>>) -> NodeQueryCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |_workspace: String, _query: Value| {
        Box::pin(async move {
            Err(raisin_error::Error::Backend(
                "Node query not yet implemented - use raisin.sql.query() for complex queries"
                    .to_string(),
            ))
        })
    })
}

// ============================================================================
// WRITE OPERATIONS
// ============================================================================

/// Create node_create callback: `raisin.nodes.create(workspace, parentPath, data)`
///
/// Creates a new node using SQL INSERT. The operation goes through QueryEngine
/// with auto-commit, ensuring consistent transaction behavior.
///
/// The `data` object should contain: name, node_type (or type), and properties.
pub fn create_node_create<S, B>(query_ctx: Arc<QueryContext<S, B>>) -> NodeCreateCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |workspace: String, parent_path: String, data: Value| {
        let ctx = query_ctx.clone();

        Box::pin(async move {
            // Parse node data from JSON
            let node = parse_node_create_data(&parent_path, data)?;

            tracing::debug!(
                workspace = %workspace,
                path = %node.path,
                node_type = %node.node_type,
                "Creating node via SQL INSERT"
            );

            // Generate and execute INSERT
            let stmt = sql_generator::generate_insert(&workspace, &node);
            ctx.execute_statement(&stmt).await?;

            // Return created node as JSON
            Ok(serde_json::to_value(node).unwrap_or_default())
        })
    })
}

/// Create node_update callback: `raisin.nodes.update(workspace, path, data)`
///
/// Updates an existing node's properties using SQL UPDATE.
/// Goes through QueryEngine with auto-commit.
pub fn create_node_update<S, B>(query_ctx: Arc<QueryContext<S, B>>) -> NodeUpdateCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |workspace: String, path: String, data: Value| {
        let ctx = query_ctx.clone();

        Box::pin(async move {
            // First, get the existing node to apply updates
            let get_stmt = sql_generator::generate_select_by_path(&workspace, &path);
            let rows = ctx.execute_query(&get_stmt).await?;

            let existing_node = rows.into_iter().next().ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Node not found: {}", path))
            })?;

            // Convert JSON to Node and apply updates
            let mut node: Node = serde_json::from_value(existing_node).map_err(|e| {
                raisin_error::Error::Internal(format!("Failed to parse node: {}", e))
            })?;

            apply_node_updates(&mut node, data)?;

            tracing::debug!(
                workspace = %workspace,
                path = %path,
                "Updating node via SQL UPDATE"
            );

            // Generate and execute UPDATE
            let stmt =
                sql_generator::generate_update_properties(&workspace, &path, &node.properties);
            ctx.execute_statement(&stmt).await?;

            Ok(serde_json::to_value(node).unwrap_or_default())
        })
    })
}

/// Create node_delete callback: `raisin.nodes.delete(workspace, path)`
///
/// Deletes a node and all its descendants using SQL DELETE.
/// Goes through QueryEngine with auto-commit.
pub fn create_node_delete<S, B>(query_ctx: Arc<QueryContext<S, B>>) -> NodeDeleteCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |workspace: String, path: String| {
        let ctx = query_ctx.clone();

        Box::pin(async move {
            tracing::debug!(
                workspace = %workspace,
                path = %path,
                "Deleting node via SQL DELETE"
            );

            // Use cascade delete to handle children
            let stmt = sql_generator::generate_delete_cascade(&workspace, &path);
            ctx.execute_statement(&stmt).await?;

            Ok(())
        })
    })
}

/// Create node_update_property callback: `raisin.nodes.updateProperty(workspace, nodePath, propertyPath, value)`
///
/// Updates a single property using SQL UPDATE with JSON merge.
/// Goes through QueryEngine with auto-commit.
pub fn create_node_update_property<S, B>(
    query_ctx: Arc<QueryContext<S, B>>,
) -> NodeUpdatePropertyCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(
        move |workspace: String, node_path: String, property_path: String, value: Value| {
            let ctx = query_ctx.clone();

            Box::pin(async move {
                // Convert JSON value to PropertyValue
                let prop_value = json_to_property_value(value)?;

                tracing::debug!(
                    workspace = %workspace,
                    node_path = %node_path,
                    property_path = %property_path,
                    "Updating property via SQL UPDATE"
                );

                // Generate and execute UPDATE
                let stmt = sql_generator::generate_update_single_property(
                    &workspace,
                    &node_path,
                    &property_path,
                    &prop_value,
                );
                ctx.execute_statement(&stmt).await?;

                Ok(())
            })
        },
    )
}

/// Create node_move callback: `raisin.nodes.move(workspace, nodePath, newParentPath)`
///
/// Moves a node to a new parent using SQL MOVE statement.
/// The node's name stays the same; only the parent path changes.
pub fn create_node_move<S, B>(query_ctx: Arc<QueryContext<S, B>>) -> NodeMoveCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(
        move |workspace: String, node_path: String, new_parent_path: String| {
            let ctx = query_ctx.clone();

            Box::pin(async move {
                // Extract node name from current path
                let node_name = node_path.split('/').next_back().ok_or_else(|| {
                    raisin_error::Error::Validation("Invalid node path".to_string())
                })?;

                // Build new path
                let new_path = if new_parent_path == "/" {
                    format!("/{}", node_name)
                } else {
                    format!("{}/{}", new_parent_path, node_name)
                };

                tracing::debug!(
                    workspace = %workspace,
                    old_path = %node_path,
                    new_path = %new_path,
                    "Moving node via SQL MOVE"
                );

                // Generate and execute MOVE
                let stmt = sql_generator::generate_move(&workspace, &node_path, &new_path);
                ctx.execute_statement(&stmt).await?;

                // Fetch the moved node to return it
                let get_stmt = sql_generator::generate_select_by_path(&workspace, &new_path);
                let rows = ctx.execute_query(&get_stmt).await?;

                let moved_node = rows.into_iter().next().ok_or_else(|| {
                    raisin_error::Error::Internal(format!(
                        "Failed to retrieve moved node at: {}",
                        new_path
                    ))
                })?;

                Ok(moved_node)
            })
        },
    )
}

// ============================================================================
// LEGACY FUNCTION SIGNATURES (for backward compatibility during transition)
// ============================================================================

/// Create node_get callback using direct storage access (legacy).
///
/// This is kept for backward compatibility. New code should use the QueryContext version.
#[deprecated(note = "Use the QueryContext-based version instead")]
pub fn create_node_get_legacy<S>(
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> NodeGetCallback
where
    S: Storage + 'static,
{
    use raisin_storage::NodeRepository;

    Arc::new(move |workspace: String, path: String| {
        let storage = storage.clone();
        let tenant = tenant_id.clone();
        let repo = repo_id.clone();
        let branch = branch.clone();
        let _auth = auth_context.clone();

        Box::pin(async move {
            let node = storage
                .nodes()
                .get_by_path(
                    StorageScope::new(&tenant, &repo, &branch, &workspace),
                    &path,
                    None,
                )
                .await?;
            Ok(node.map(|n| serde_json::to_value(n).unwrap_or_default()))
        })
    })
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Parse node creation data from JSON.
pub fn parse_node_create_data(parent_path: &str, data: Value) -> raisin_error::Result<Node> {
    let name = data
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| raisin_error::Error::Validation("Missing 'name' field".to_string()))?;

    let node_type = data
        .get("node_type")
        .or_else(|| data.get("type"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            raisin_error::Error::Validation("Missing 'node_type' or 'type' field".to_string())
        })?;

    let path = if parent_path == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", parent_path, name)
    };

    let mut node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        path,
        node_type: node_type.to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Default::default()
    };

    // Parse properties if provided
    if let Some(props) = data.get("properties").and_then(|v| v.as_object()) {
        for (key, value) in props {
            node.properties
                .insert(key.clone(), json_to_property_value(value.clone())?);
        }
    }

    Ok(node)
}

/// Apply updates from JSON data to an existing node.
pub fn apply_node_updates(node: &mut Node, data: Value) -> raisin_error::Result<()> {
    // Update properties if provided
    if let Some(props) = data.get("properties").and_then(|v| v.as_object()) {
        for (key, value) in props {
            node.properties
                .insert(key.clone(), json_to_property_value(value.clone())?);
        }
    }

    // Update timestamp
    node.updated_at = Some(chrono::Utc::now());

    Ok(())
}

/// Convert a JSON Value to a PropertyValue.
pub fn json_to_property_value(value: Value) -> raisin_error::Result<PropertyValue> {
    match value {
        Value::Null => Ok(PropertyValue::Null),
        Value::Bool(b) => Ok(PropertyValue::Boolean(b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(PropertyValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(PropertyValue::Float(f))
            } else {
                Err(raisin_error::Error::Validation(
                    "Invalid number".to_string(),
                ))
            }
        }
        Value::String(s) => Ok(PropertyValue::String(s)),
        Value::Array(arr) => {
            let items: raisin_error::Result<Vec<_>> =
                arr.into_iter().map(json_to_property_value).collect();
            Ok(PropertyValue::Array(items?))
        }
        Value::Object(obj) => {
            // Treat objects with raisin:ref as references
            if let Some(id) = obj.get("raisin:ref").and_then(|v| v.as_str()) {
                let workspace = obj
                    .get("raisin:workspace")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let path = obj
                    .get("raisin:path")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();

                return Ok(PropertyValue::Reference(RaisinReference {
                    id: id.to_string(),
                    workspace,
                    path,
                }));
            }

            let mut map = std::collections::HashMap::new();
            for (k, v) in obj {
                map.insert(k, json_to_property_value(v)?);
            }
            Ok(PropertyValue::Object(map))
        }
    }
}
