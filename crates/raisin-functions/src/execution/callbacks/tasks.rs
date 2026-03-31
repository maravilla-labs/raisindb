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

//! Task operation callbacks for function execution.
//!
//! These callbacks implement the `raisin.tasks.*` API available to JavaScript functions.
//! Creates human tasks (InboxTask nodes) in user inboxes.

use std::sync::Arc;

use chrono::Utc;
use raisin_core::services::node_service::NodeService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::{TaskCompleteCallback, TaskCreateCallback, TaskQueryCallback, TaskUpdateCallback};

/// Create task_create callback: `raisin.tasks.create(request)`
///
/// Creates a human task (raisin:InboxTask) in the assignee's inbox.
///
/// Required request fields:
/// - `task_type`: "approval" | "input" | "review" | "action"
/// - `title`: Task title string
/// - `assignee`: User path (e.g., "/users/manager" or "users/manager")
///
/// Optional request fields:
/// - `description`: Task description
/// - `options`: Array of { value, label, style } for approval tasks
/// - `input_schema`: JSON schema for input tasks
/// - `due_in_seconds`: Task due time in seconds from now
/// - `priority`: 1-5, where 5 is highest
///
/// Returns: { task_id, task_path }
pub fn create_task_create<S>(
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> TaskCreateCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |request: Value| {
        let storage = storage.clone();
        let tenant = tenant_id.clone();
        let repo = repo_id.clone();
        let branch = branch.clone();
        let _auth = auth_context.clone();

        Box::pin(async move {
            // Validate required fields
            let task_type = request
                .get("task_type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    raisin_error::Error::Validation("task_type is required".to_string())
                })?;

            let title = request
                .get("title")
                .and_then(|v| v.as_str())
                .ok_or_else(|| raisin_error::Error::Validation("title is required".to_string()))?;

            let assignee = request
                .get("assignee")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    raisin_error::Error::Validation("assignee is required".to_string())
                })?;

            // Validate task_type
            if !["approval", "input", "review", "action"].contains(&task_type) {
                return Err(raisin_error::Error::Validation(format!(
                    "Invalid task_type: {}. Must be one of: approval, input, review, action",
                    task_type
                )));
            }

            // Generate task ID and path
            let task_id = Uuid::new_v4().to_string();
            let timestamp = Utc::now().timestamp();

            // Normalize assignee path (remove leading slash for path construction)
            let assignee_normalized = assignee.trim_start_matches('/');

            // Task name for the node
            let task_name = format!("task-{}-{}", &task_id[..8], timestamp);

            // Build task path: users/{assignee}/inbox/{task_name}
            let task_path = format!("{}/inbox/{}", assignee_normalized, task_name);

            // Build the Node struct
            let mut node = Node {
                id: task_id.clone(),
                name: task_name,
                path: task_path.clone(),
                node_type: "raisin:InboxTask".to_string(),
                created_at: Some(Utc::now()),
                ..Default::default()
            };

            // Add required properties
            node.properties.insert(
                "task_type".to_string(),
                PropertyValue::String(task_type.to_string()),
            );
            node.properties.insert(
                "title".to_string(),
                PropertyValue::String(title.to_string()),
            );
            node.properties.insert(
                "status".to_string(),
                PropertyValue::String("pending".to_string()),
            );

            // Add optional properties
            if let Some(description) = request.get("description").and_then(|v| v.as_str()) {
                node.properties.insert(
                    "description".to_string(),
                    PropertyValue::String(description.to_string()),
                );
            }

            if let Some(options) = request.get("options") {
                node.properties.insert(
                    "options".to_string(),
                    PropertyValue::Object(
                        serde_json::from_value(options.clone()).unwrap_or_default(),
                    ),
                );
            }

            if let Some(input_schema) = request.get("input_schema") {
                node.properties.insert(
                    "input_schema".to_string(),
                    PropertyValue::Object(
                        serde_json::from_value(input_schema.clone()).unwrap_or_default(),
                    ),
                );
            }

            if let Some(priority) = request.get("priority").and_then(|v| v.as_i64()) {
                node.properties
                    .insert("priority".to_string(), PropertyValue::Integer(priority));
            }

            if let Some(due_in_seconds) = request.get("due_in_seconds").and_then(|v| v.as_i64()) {
                let due_at = Utc::now() + chrono::Duration::seconds(due_in_seconds);
                node.properties
                    .insert("due_at".to_string(), PropertyValue::Date(due_at.into()));
            }

            // Create NodeService for the users workspace
            let svc =
                NodeService::new_with_context(storage, tenant, repo, branch, "users".to_string());

            // Create the task node
            svc.create(node.clone()).await?;

            // Return task info
            Ok(json!({
                "task_id": task_id,
                "task_path": task_path,
            }))
        })
    })
}

/// Create task_update callback: `raisin.tasks.update(taskId, updates)`
///
/// Updates an existing task by ID.
///
/// Updates can include:
/// - `status`: New status ("pending", "completed", "expired", "cancelled")
/// - `response`: User's response object
/// - `priority`: New priority (1-5)
///
/// Returns: Updated task as JSON
pub fn create_task_update<S>(
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    _auth_context: Option<AuthContext>,
) -> TaskUpdateCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |task_id: String, updates: Value| {
        let storage = storage.clone();
        let tenant = tenant_id.clone();
        let repo = repo_id.clone();
        let branch = branch.clone();

        Box::pin(async move {
            // Create NodeService for the raisin:access_control workspace
            let svc = NodeService::new_with_context(
                storage,
                tenant,
                repo,
                branch,
                "raisin:access_control".to_string(),
            );

            // Get the task by ID
            let task = svc.get(&task_id).await?.ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Task not found: {}", task_id))
            })?;

            // Validate status if provided
            if let Some(status) = updates.get("status").and_then(|v| v.as_str()) {
                if !["pending", "completed", "expired", "cancelled"].contains(&status) {
                    return Err(raisin_error::Error::Validation(format!(
                        "Invalid status: {}. Must be one of: pending, completed, expired, cancelled",
                        status
                    )));
                }
            }

            // Build updated properties
            let mut updated_props = task.properties.clone();

            if let Some(status) = updates.get("status").and_then(|v| v.as_str()) {
                updated_props.insert(
                    "status".to_string(),
                    PropertyValue::String(status.to_string()),
                );
            }

            if let Some(response) = updates.get("response") {
                updated_props.insert(
                    "response".to_string(),
                    PropertyValue::Object(
                        serde_json::from_value(response.clone()).unwrap_or_default(),
                    ),
                );
            }

            if let Some(priority) = updates.get("priority").and_then(|v| v.as_i64()) {
                updated_props.insert("priority".to_string(), PropertyValue::Integer(priority));
            }

            // Create updated node
            let updated_node = Node {
                properties: updated_props,
                ..task
            };

            // Update via create (upsert behavior)
            svc.create(updated_node.clone()).await.ok(); // Ignore error if exists

            // Return updated task info
            Ok(json!({
                "id": task_id,
                "status": updates.get("status").and_then(|s| s.as_str()).unwrap_or("pending"),
                "updated": true
            }))
        })
    })
}

/// Create task_complete callback: `raisin.tasks.complete(taskId, response)`
///
/// Marks a task as completed with the given response.
///
/// Returns: Completed task as JSON
pub fn create_task_complete<S>(
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    _auth_context: Option<AuthContext>,
) -> TaskCompleteCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |task_id: String, response: Value| {
        let storage = storage.clone();
        let tenant = tenant_id.clone();
        let repo = repo_id.clone();
        let branch = branch.clone();

        Box::pin(async move {
            // Create NodeService for the raisin:access_control workspace
            let svc = NodeService::new_with_context(
                storage,
                tenant,
                repo,
                branch,
                "raisin:access_control".to_string(),
            );

            // Get the task by ID
            let task = svc.get(&task_id).await?.ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Task not found: {}", task_id))
            })?;

            // Build updated properties
            let mut updated_props = task.properties.clone();
            updated_props.insert(
                "status".to_string(),
                PropertyValue::String("completed".to_string()),
            );
            updated_props.insert(
                "response".to_string(),
                PropertyValue::Object(serde_json::from_value(response.clone()).unwrap_or_default()),
            );
            updated_props.insert(
                "responded_at".to_string(),
                PropertyValue::Date(Utc::now().into()),
            );

            // Create updated node
            let updated_node = Node {
                properties: updated_props,
                ..task
            };

            // Update via create (upsert behavior)
            svc.create(updated_node.clone()).await.ok();

            // Return completed task info
            Ok(json!({
                "id": task_id,
                "status": "completed",
                "responded_at": Utc::now().to_rfc3339()
            }))
        })
    })
}

/// Create task_query callback: `raisin.tasks.query(query)`
///
/// Queries tasks based on filter criteria.
///
/// Query parameters:
/// - `assignee`: User path to filter by
/// - `status`: Task status to filter by
/// - `due_before`: ISO date string to filter tasks due before
/// - `limit`: Maximum results
///
/// Returns: Array of matching tasks
pub fn create_task_query<S>(
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    _auth_context: Option<AuthContext>,
) -> TaskQueryCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |query: Value| {
        let storage = storage.clone();
        let tenant = tenant_id.clone();
        let repo = repo_id.clone();
        let branch = branch.clone();

        Box::pin(async move {
            // Create NodeService for the raisin:access_control workspace
            let svc = NodeService::new_with_context(
                storage,
                tenant,
                repo,
                branch,
                "raisin:access_control".to_string(),
            );

            // Get limit from query
            let limit = query.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;

            // Get assignee filter
            let assignee = query.get("assignee").and_then(|v| v.as_str());
            let status_filter = query.get("status").and_then(|v| v.as_str());

            // Query tasks by listing nodes of type InboxTask
            let all_tasks = svc.list_by_type("raisin:InboxTask").await?;

            // Filter and convert
            let filtered: Vec<Value> = all_tasks
                .into_iter()
                .filter(|task| {
                    // Filter by assignee path
                    if let Some(assignee_path) = assignee {
                        let assignee_normalized = assignee_path.trim_start_matches('/');
                        if !task.path.starts_with(assignee_normalized) {
                            return false;
                        }
                    }

                    // Filter by status
                    if let Some(status) = status_filter {
                        if let Some(PropertyValue::String(task_status)) =
                            task.properties.get("status")
                        {
                            if task_status != status {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }

                    true
                })
                .take(limit)
                .map(|task| serde_json::to_value(task).unwrap_or_default())
                .collect();

            Ok(filtered)
        })
    })
}
