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
use raisin_flow_runtime::service::TaskCompleter;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::{json, Value};
use uuid::Uuid;

use super::flows::JobQueueFlowScheduler;
use crate::api::{TaskCompleteCallback, TaskCreateCallback, TaskQueryCallback, TaskUpdateCallback};

/// Derive the [`TaskCompleter`] identity from the executing function's auth
/// context, so the function-side `raisin.tasks.complete` enforces the SAME
/// assignee validation as the HTTP inbox path (`complete_inbox_task`).
///
/// - `Some(user)`: a real caller. `is_admin` is true for system/`admin`/
///   `system_admin` roles (they may complete any task); otherwise the caller
///   may only complete a task assigned to their own home path (or whose last
///   path segment is their id) - `complete_task` gates on this.
/// - `None`: the documented function "system context" (no RLS - see
///   `execute_function`). System flows, triggers and agents run this way and
///   complete tasks as an admin. A real end-user invocation always carries a
///   `Some(user)` context, so user-facing validation is preserved.
fn completer_from_auth(auth: &Option<AuthContext>) -> TaskCompleter {
    match auth {
        Some(a) => {
            let is_admin =
                a.is_system || a.roles.iter().any(|r| r == "system_admin" || r == "admin");
            let id = a
                .user_id
                .clone()
                .or_else(|| a.email.clone())
                .unwrap_or_else(|| "function".to_string());
            TaskCompleter {
                id,
                home: a.home.clone(),
                is_admin,
            }
        }
        None => TaskCompleter {
            id: "system".to_string(),
            home: None,
            is_admin: true,
        },
    }
}

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

            // Build task path: /{assignee}/inbox/{task_name}. Leading slash so
            // the node is stored under the same canonical path as user homes
            // and flow-created inbox tasks (e.g. "/users/alice/inbox/...").
            let task_path = format!("/{}/inbox/{}", assignee_normalized, task_name);

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
            // Store the assignee HOME path as a property (normalized with a
            // leading slash) so complete_task / get_inbox_task can authorize the
            // caller and list_inbox_tasks can filter by assignee. Without this a
            // non-admin assignee cannot complete their own standalone task (the
            // authz reads this property, not the node path). Mirrors the
            // human_task step, which also stores `assignee`.
            node.properties.insert(
                "assignee".to_string(),
                PropertyValue::String(format!("/{}", assignee_normalized)),
            );

            // Add optional properties
            if let Some(description) = request.get("description").and_then(|v| v.as_str()) {
                node.properties.insert(
                    "description".to_string(),
                    PropertyValue::String(description.to_string()),
                );
            }

            if let Some(options) = request.get("options") {
                // `options` is a JSON ARRAY ([{value,label,style}, ...]); it must
                // be stored as PropertyValue::Array. The previous code forced
                // PropertyValue::Object(from_value::<Map>(..)) which fails for an
                // array and silently collapsed options to an empty {}.
                let items: Vec<PropertyValue> =
                    serde_json::from_value(options.clone()).unwrap_or_default();
                node.properties
                    .insert("options".to_string(), PropertyValue::Array(items));
            }

            if let Some(input_schema) = request.get("input_schema") {
                node.properties.insert(
                    "input_schema".to_string(),
                    PropertyValue::Object(
                        serde_json::from_value(input_schema.clone()).unwrap_or_default(),
                    ),
                );
            }

            // Arbitrary structured payload for the task UI (declared `data`
            // Object on raisin:InboxTask v4). Without this the strict type
            // silently dropped `data` on write, so the inbox conflict tree —
            // which renders task.data.changes — had nothing to render. Stored as
            // an Object so nested arrays (e.g. `changes: [SyncEntry]`) round-trip.
            if let Some(data) = request.get("data") {
                if !data.is_null() {
                    node.properties.insert(
                        "data".to_string(),
                        PropertyValue::Object(
                            serde_json::from_value(data.clone()).unwrap_or_default(),
                        ),
                    );
                }
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

            // Create NodeService for the inbox workspace. User homes (and
            // their inbox children) live in raisin:access_control - the same
            // workspace task_update/complete/query use.
            //
            // Placing a task in a user's inbox is a privileged framework
            // operation: the assignee is usually NOT the calling function's
            // principal, and the access_control workspace's RLS would otherwise
            // deny the create (NodeService denies creates when no auth context
            // is set). Use the system context to bypass RLS, mirroring the
            // human_task flow step, which creates inbox tasks with full
            // privileges via the flow node-creator.
            let svc = NodeService::new_with_context(
                storage,
                tenant,
                repo,
                branch,
                "raisin:access_control".to_string(),
            )
            .with_auth(AuthContext::system());

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
            // Create NodeService for the inbox workspace. Updating an inbox
            // task is a privileged framework operation (the caller is usually
            // NOT the task's principal), so use the system context to bypass
            // RLS - mirroring create_task_create. Without it, NodeService
            // denies the update (no auth context => RLS deny), which the old
            // `create(..).ok()` silently swallowed (making update a no-op).
            let svc = NodeService::new_with_context(
                storage,
                tenant,
                repo,
                branch,
                "raisin:access_control".to_string(),
            )
            .with_auth(AuthContext::system());

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

            // Persist via update (NOT create-upsert): create on an existing
            // path now conflicts (post path-uniqueness fix) and the old
            // `.ok()` swallowed that, silently dropping the update.
            svc.update_node(updated_node).await?;

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
/// Marks a task completed AND resumes the owning flow (if any), routed through
/// the SAME service the HTTP inbox uses
/// (`raisin_flow_runtime::service::complete_task`). That service:
/// 1. validates the caller (derived from the function's auth context) is the
///    task's assignee (or an admin) - see [`completer_from_auth`];
/// 2. guards idempotency (an already-completed task returns an error);
/// 3. persists the completion via `nodes().update` - NOT a create-upsert,
///    which now conflicts on the existing path and, via the old `.ok()`,
///    silently swallowed the error so the task never actually changed;
/// 4. queues the flow resume on the unified job queue with the response as
///    `__human_response`, so a parked `human_task` step actually advances.
///
/// Requires the job system (`job_registry` + `job_data_store`); completion is
/// inseparable from resuming the flow.
///
/// Returns (preserving the historical `{ id, status, responded_at }` shape and
/// adding the resumed flow): `{ id, task_id, task_path, status, responded_at,
/// flow }` where `flow` is `{ instance_id, job_id }` for a flow-owned task or
/// `null` for a standalone task.
pub fn create_task_complete<S>(
    storage: Arc<S>,
    job_registry: Arc<raisin_storage::jobs::JobRegistry>,
    job_data_store: Arc<raisin_rocksdb::JobDataStore>,
    repo_id: String,
    auth_context: Option<AuthContext>,
) -> TaskCompleteCallback
where
    S: Storage + TransactionalStorage + 'static,
{
    Arc::new(move |task_id: String, response: Value| {
        let storage = storage.clone();
        let scheduler = JobQueueFlowScheduler::new(job_registry.clone(), job_data_store.clone());
        let repo = repo_id.clone();
        let completer = completer_from_auth(&auth_context);

        Box::pin(async move {
            let result = raisin_flow_runtime::service::complete_task(
                storage.as_ref(),
                &scheduler,
                &repo,
                &task_id,
                &completer,
                response,
            )
            .await
            .map_err(|e| {
                raisin_error::Error::Backend(format!(
                    "Failed to complete task '{}': {}",
                    task_id, e
                ))
            })?;

            // `flow` is null for standalone (non-flow) tasks.
            let flow = serde_json::to_value(&result.flow).unwrap_or(Value::Null);
            Ok(json!({
                "id": result.task_id,
                "task_id": result.task_id,
                "task_path": result.task_path,
                "status": result.status,
                "responded_at": Utc::now().to_rfc3339(),
                "flow": flow,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The `data` payload passed to raisin.tasks.create must round-trip into a
    /// PropertyValue::Object with its nested arrays intact — this is exactly the
    /// conversion create_task_create performs, and the reason the linked-site
    /// conflict tree (which reads task.data.changes) had nothing to render before
    /// the `data` slot existed. Mirrors the callback's insert logic.
    #[test]
    fn task_data_object_preserves_nested_changes_array() {
        let data = json!({
            "kind": "link_conflict",
            "copy_root": "/gov-copy",
            "workspace": "stories",
            "change_key": "abc123",
            "changes": [
                { "copy_path": "/gov-copy/about", "source_path": "/gov-src/about",
                  "source_id": "id-1", "verdict": "await-owner",
                  "overrides": ["description"] },
            ],
        });

        // Exactly what create_task_create stores for the `data` property.
        let map: HashMap<String, PropertyValue> =
            serde_json::from_value(data.clone()).unwrap_or_default();
        let stored = PropertyValue::Object(map);

        let PropertyValue::Object(obj) = &stored else {
            panic!("data did not deserialize to an Object");
        };
        // Scalars survive.
        assert!(matches!(obj.get("copy_root"), Some(PropertyValue::String(s)) if s == "/gov-copy"));
        // The nested `changes` array is preserved (the renderable tree) — the
        // old strict schema dropped the whole `data` bag, so this was empty.
        match obj.get("changes") {
            Some(PropertyValue::Array(items)) => {
                assert_eq!(items.len(), 1, "changes array must round-trip with 1 entry");
                let PropertyValue::Object(entry) = &items[0] else {
                    panic!("change entry is not an Object");
                };
                assert!(
                    matches!(entry.get("copy_path"), Some(PropertyValue::String(s)) if s == "/gov-copy/about"),
                    "change entry must carry copy_path (SyncEntry shape)"
                );
            }
            other => panic!("changes did not round-trip as an Array: {:?}", other),
        }
    }

    /// A null `data` is skipped (never stored), matching the callback guard.
    #[test]
    fn task_data_null_is_skipped() {
        let data = Value::Null;
        assert!(
            data.is_null(),
            "null data must be treated as absent by create_task_create"
        );
    }

    // -- completer_from_auth: assignee-validation preservation at the binding --
    //
    // `create_task_complete` routes through `service::complete_task`, which
    // gates completion on `TaskCompleter::matches_assignee` (tested in the
    // flow-runtime service). These tests pin the OTHER half: the binding must
    // derive a completer that carries the caller's real identity for a normal
    // user (so validation applies) and only bypasses for admin/system.

    /// A regular (non-admin) user maps to a non-admin completer carrying their
    /// id + home, so `complete_task` validates them against the assignee.
    #[test]
    fn completer_from_regular_user_preserves_identity_and_is_not_admin() {
        let mut a = AuthContext::for_user("cdc46e95-425d-4816-9da1-9a61776bbcf3");
        a.home = Some("/users/internal/admin-at-example-local".to_string());
        let c = completer_from_auth(&Some(a));
        assert_eq!(c.id, "cdc46e95-425d-4816-9da1-9a61776bbcf3");
        assert_eq!(
            c.home.as_deref(),
            Some("/users/internal/admin-at-example-local")
        );
        assert!(
            !c.is_admin,
            "a regular user must NOT bypass assignee validation"
        );
    }

    /// The `admin` / `system_admin` roles (and `is_system`) grant admin bypass.
    #[test]
    fn completer_from_admin_or_system_bypasses() {
        let mut role_admin = AuthContext::for_user("u1");
        role_admin.roles = vec!["admin".to_string()];
        assert!(completer_from_auth(&Some(role_admin)).is_admin);

        let mut sys_role = AuthContext::for_user("u2");
        sys_role.roles = vec!["system_admin".to_string()];
        assert!(completer_from_auth(&Some(sys_role)).is_admin);

        // AuthContext::system() sets is_system = true.
        assert!(completer_from_auth(&Some(AuthContext::system())).is_admin);
    }

    /// A user without user_id falls back to email for the completer id.
    #[test]
    fn completer_falls_back_to_email() {
        let mut a = AuthContext::for_user("ignored");
        a.user_id = None;
        a.email = Some("manager@example.local".to_string());
        assert_eq!(completer_from_auth(&Some(a)).id, "manager@example.local");
    }

    /// A `None` auth context is the documented function system context: it
    /// completes as an admin (system flows / triggers / agents drive this).
    #[test]
    fn completer_from_none_is_system_admin() {
        let c = completer_from_auth(&None);
        assert_eq!(c.id, "system");
        assert!(c.home.is_none());
        assert!(c.is_admin, "the system context (None) completes as admin");
    }
}
