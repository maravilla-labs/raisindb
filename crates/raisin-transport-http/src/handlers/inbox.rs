// SPDX-License-Identifier: BSL-1.1

//! Inbox task endpoints: list, get, and complete human tasks.
//!
//! Inbox tasks (`raisin:InboxTask` nodes) are the human-in-the-loop
//! primitive: workflows create them for approvals/input/review, users (or
//! AI agents) complete them, and completion resumes the waiting flow.
//!
//! Routes:
//! - `GET  /api/inbox/{repo}`                      - list the caller's tasks
//! - `GET  /api/inbox/{repo}/tasks/{task_id}`      - get one task
//! - `POST /api/inbox/{repo}/tasks/{task_id}/complete` - complete a task

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use serde::Deserialize;
use serde_json::Value;

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

/// Query params for listing inbox tasks.
#[derive(Debug, Deserialize)]
pub struct ListInboxQuery {
    /// Filter by status (pending | completed | expired | cancelled)
    pub status: Option<String>,
    /// Admins may list another principal's inbox
    pub assignee: Option<String>,
}

/// Request body for completing a task.
#[derive(Debug, Deserialize)]
pub struct CompleteTaskRequest {
    /// The response payload - e.g. `{ "action": "approve", "comment": "ok" }`
    /// for approval tasks, or the input value for input tasks.
    #[serde(default)]
    pub response: Value,
}

fn is_admin(auth: &AuthContext) -> bool {
    auth.is_system
        || auth
            .roles
            .iter()
            .any(|r| r == "system_admin" || r == "admin")
}

/// Resolve the caller's identity, rejecting anonymous requests.
fn require_caller(
    auth_context: &Option<Extension<AuthContext>>,
) -> Result<(String, Option<String>, bool), ApiError> {
    let auth = auth_context
        .as_ref()
        .map(|ext| &ext.0)
        .filter(|a| !a.is_anonymous)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED",
                "Authentication required for inbox access",
            )
        })?;

    let id = auth
        .user_id
        .clone()
        .or_else(|| auth.email.clone())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED",
                "Authenticated user has no identity",
            )
        })?;

    Ok((id, auth.home.clone(), is_admin(auth)))
}

/// List the caller's inbox tasks.
#[cfg(feature = "storage-rocksdb")]
pub async fn list_inbox_tasks(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path(repo): Path<String>,
    auth_context: Option<Extension<AuthContext>>,
    Query(query): Query<ListInboxQuery>,
) -> Result<Json<Value>, ApiError> {
    let (caller_id, home, admin) = require_caller(&auth_context)?;

    // Default to the caller's own inbox; admins may inspect another
    // principal's inbox or pass `assignee=*` for an overview of ALL tasks
    let assignee: Option<String> = match query.assignee {
        Some(other) if other == "*" => {
            if !admin {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "FORBIDDEN",
                    "Only admins may list all inbox tasks",
                ));
            }
            None
        }
        Some(other) => {
            let own = home.as_deref().map(|h| h.trim_matches('/').to_string());
            if !admin && own.as_deref() != Some(other.trim_matches('/')) {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "FORBIDDEN",
                    "Only admins may list another principal's inbox",
                ));
            }
            Some(other)
        }
        None => Some(
            home.clone()
                .unwrap_or_else(|| format!("/users/{}", caller_id)),
        ),
    };

    let tasks = raisin_flow_runtime::service::list_inbox_tasks(
        state.storage.as_ref(),
        &tenant_info.tenant_id,
        &repo,
        assignee.as_deref(),
        query.status.as_deref(),
    )
    .await?;

    Ok(Json(serde_json::json!({
        "assignee": assignee.unwrap_or_else(|| "*".to_string()),
        "count": tasks.len(),
        "tasks": tasks,
    })))
}

/// Get a single inbox task.
#[cfg(feature = "storage-rocksdb")]
pub async fn get_inbox_task(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo, task_id)): Path<(String, String)>,
    auth_context: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    let (caller_id, home, admin) = require_caller(&auth_context)?;

    let task =
        raisin_flow_runtime::service::get_inbox_task(state.storage.as_ref(), &tenant_info.tenant_id, &repo, &task_id)
            .await?;

    // Only the assignee (or an admin) may read a task
    if !admin {
        let assignee = task
            .get("assignee")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim_matches('/')
            .to_string();
        let own_home = home.as_deref().map(|h| h.trim_matches('/').to_string());
        let is_assignee = own_home.as_deref() == Some(assignee.as_str())
            || assignee.rsplit('/').next() == Some(caller_id.as_str());
        if !is_assignee {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "FORBIDDEN",
                "Task is assigned to another principal",
            ));
        }
    }

    Ok(Json(task))
}

/// Complete an inbox task. Validates the caller is the assignee, marks the
/// task completed, and resumes the owning flow.
#[cfg(feature = "storage-rocksdb")]
pub async fn complete_inbox_task(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo, task_id)): Path<(String, String)>,
    auth_context: Option<Extension<AuthContext>>,
    Json(req): Json<CompleteTaskRequest>,
) -> Result<Json<Value>, ApiError> {
    let (caller_id, home, admin) = require_caller(&auth_context)?;

    let scheduler = raisin_rocksdb::get_flow_job_scheduler(&state.rocksdb_storage)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let completer = raisin_flow_runtime::service::TaskCompleter {
        id: caller_id,
        home,
        is_admin: admin,
    };

    let result = raisin_flow_runtime::service::complete_task(
        state.storage.as_ref(),
        scheduler,
        &tenant_info.tenant_id,
        &repo,
        &task_id,
        &completer,
        req.response,
    )
    .await?;

    Ok(Json(serde_json::to_value(result).unwrap_or(Value::Null)))
}

// ---------------------------------------------------------------------------
// Non-RocksDB stubs
// ---------------------------------------------------------------------------

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn list_inbox_tasks(
    State(_state): State<AppState>,
    Path(_repo): Path<String>,
    _auth_context: Option<Extension<AuthContext>>,
    Query(_query): Query<ListInboxQuery>,
) -> Result<Json<Value>, ApiError> {
    Err(ApiError::internal("Inbox requires RocksDB backend"))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn get_inbox_task(
    State(_state): State<AppState>,
    Path((_repo, _task_id)): Path<(String, String)>,
    _auth_context: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    Err(ApiError::internal("Inbox requires RocksDB backend"))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn complete_inbox_task(
    State(_state): State<AppState>,
    Path((_repo, _task_id)): Path<(String, String)>,
    _auth_context: Option<Extension<AuthContext>>,
    Json(_req): Json<CompleteTaskRequest>,
) -> Result<Json<Value>, ApiError> {
    Err(ApiError::internal("Inbox requires RocksDB backend"))
}
