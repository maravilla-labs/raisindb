// SPDX-License-Identifier: BSL-1.1
//! Shared types and helpers for command execution.

use axum::http::StatusCode;
use axum::Json;
use raisin_core::NodeService;
use raisin_models::auth::AuthContext;
use raisin_storage::Storage;

use crate::error::ApiError;
use crate::state::AppState;
use crate::types::CommandBody;

/// Result type for command execution.
pub type CommandResult = Result<(StatusCode, Json<serde_json::Value>), ApiError>;

/// Context for command execution, containing all necessary state and parameters.
pub struct CommandContext<'a, S: Storage> {
    pub state: &'a AppState,
    pub tenant_id: &'a str,
    pub repository: &'a str,
    pub branch: &'a str,
    pub ws: &'a str,
    pub path: &'a str,
    pub params: CommandBody,
    pub auth: Option<AuthContext>,
    pub nodes_svc: NodeService<S>,
    pub branch_head: Option<u64>,
}

impl<'a, S: Storage> CommandContext<'a, S> {
    /// Get the actor from params or auth context, defaulting to "system".
    pub fn get_actor(&self) -> String {
        self.params.actor.clone().unwrap_or_else(|| {
            self.auth
                .as_ref()
                .map(|ctx| ctx.actor_id())
                .unwrap_or_else(|| "system".to_string())
        })
    }

    /// Create a success response with JSON body.
    pub fn ok_json(value: serde_json::Value) -> CommandResult {
        Ok((StatusCode::OK, Json(value)))
    }

    /// Create an empty success response.
    pub fn ok_empty() -> CommandResult {
        Ok((StatusCode::OK, Json(serde_json::json!({}))))
    }

    /// Create a no-content response.
    pub fn no_content() -> CommandResult {
        Ok((StatusCode::NO_CONTENT, Json(serde_json::json!({}))))
    }

    /// Create a committed response with revision.
    pub fn committed(revision: u64) -> CommandResult {
        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "revision": revision,
                "committed": true
            })),
        ))
    }

    /// Create a committed response with revision and operation count.
    pub fn committed_with_count(revision: u64, operations_count: usize) -> CommandResult {
        Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "revision": revision,
                "operations_count": operations_count
            })),
        ))
    }
}
