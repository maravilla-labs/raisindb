// SPDX-License-Identifier: BSL-1.1

//! Production-grade error handling for HTTP API
//!
//! Provides structured error responses with:
//! - Machine-readable error codes
//! - Human-friendly messages
//! - Optional technical details
//! - Field-level validation errors
//! - Timestamps for debugging

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Deref, DerefMut};

/// Inner data for API error responses (boxed to keep `Result<T, ApiError>` small)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorInner {
    /// Machine-readable error code (e.g., "NODE_NOT_FOUND")
    pub code: String,

    /// Human-friendly error message
    pub message: String,

    /// Technical details (e.g., exception message, stack trace in dev mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,

    /// Field name for validation errors
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,

    /// ISO 8601 timestamp
    pub timestamp: String,

    /// HTTP status code (not serialized, used for response)
    #[serde(skip)]
    pub status: StatusCode,
}

/// Structured API error response
///
/// Uses `Box<ApiErrorInner>` internally to keep `Result<T, ApiError>` small
/// (avoids clippy::result_large_err).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApiError(Box<ApiErrorInner>);

impl Deref for ApiError {
    type Target = ApiErrorInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ApiError {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl ApiError {
    /// Consume the error and return the inner data
    pub fn into_inner(self) -> ApiErrorInner {
        *self.0
    }

    /// Consume the error and return just the message
    pub fn into_message(self) -> String {
        self.0.message
    }

    /// Create a new API error
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self(Box::new(ApiErrorInner {
            code: code.into(),
            message: message.into(),
            details: None,
            field: None,
            timestamp: Utc::now().to_rfc3339(),
            status,
        }))
    }

    /// Add technical details to the error
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// Add field information for validation errors
    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    // === Not Found Errors (404) ===

    pub fn node_not_found(path: impl Into<String>) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "NODE_NOT_FOUND",
            format!("Node not found at path: {}", path.into()),
        )
    }

    pub fn branch_not_found(branch: impl Into<String>) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "BRANCH_NOT_FOUND",
            format!("Branch '{}' not found", branch.into()),
        )
    }

    pub fn tag_not_found(tag: impl Into<String>) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "TAG_NOT_FOUND",
            format!("Tag '{}' not found", tag.into()),
        )
    }

    pub fn repository_not_found(repo: impl Into<String>) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "REPOSITORY_NOT_FOUND",
            format!("Repository '{}' not found", repo.into()),
        )
    }

    pub fn workspace_not_found(workspace: impl Into<String>) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "WORKSPACE_NOT_FOUND",
            format!("Workspace '{}' not found", workspace.into()),
        )
    }

    pub fn node_type_not_found(node_type: impl Into<String>) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "NODE_TYPE_NOT_FOUND",
            format!("NodeType '{}' not found", node_type.into()),
        )
    }

    pub fn archetype_not_found(archetype: impl Into<String>) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "ARCHETYPE_NOT_FOUND",
            format!("Archetype '{}' not found", archetype.into()),
        )
    }

    pub fn element_type_not_found(element_type: impl Into<String>) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "ELEMENT_TYPE_NOT_FOUND",
            format!("Element type '{}' not found", element_type.into()),
        )
    }

    pub fn revision_not_found(revision: &raisin_hlc::HLC) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "REVISION_NOT_FOUND",
            format!("Revision {} not found", revision),
        )
    }

    /// Generic not found error for cases not covered by specific helpers
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "NOT_FOUND", message)
    }

    // === Validation Errors (400) ===

    pub fn invalid_node_type(node_type: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_NODE_TYPE",
            format!("Invalid node type: {}", node_type.into()),
        )
    }

    pub fn invalid_branch_name(name: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_BRANCH_NAME",
            format!("Invalid branch name: {}", name.into()),
        )
    }

    pub fn invalid_revision_number() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_REVISION_NUMBER",
            "Revision number must be a positive integer",
        )
    }

    pub fn node_already_published(path: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "NODE_ALREADY_PUBLISHED",
            format!("Cannot modify published node: {}", path.into()),
        )
    }

    pub fn validation_failed(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "VALIDATION_FAILED", message)
    }

    pub fn missing_required_field(field: impl Into<String>) -> Self {
        let field_name = field.into();
        Self::new(
            StatusCode::BAD_REQUEST,
            "MISSING_REQUIRED_FIELD",
            format!("Missing required field: {}", field_name),
        )
        .with_field(field_name)
    }

    pub fn invalid_json(details: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "INVALID_JSON",
            "Request body contains invalid JSON",
        )
        .with_details(details)
    }

    pub fn payload_too_large(size: usize, max: usize) -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
            format!("Payload size {} exceeds maximum {}", size, max),
        )
    }

    // === Conflict Errors (409) ===

    pub fn branch_already_exists(branch: impl Into<String>) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "BRANCH_ALREADY_EXISTS",
            format!("Branch '{}' already exists", branch.into()),
        )
    }

    pub fn tag_already_exists(tag: impl Into<String>) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "TAG_ALREADY_EXISTS",
            format!("Tag '{}' already exists", tag.into()),
        )
    }

    pub fn repository_already_exists(repo: impl Into<String>) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "REPOSITORY_ALREADY_EXISTS",
            format!("Repository '{}' already exists", repo.into()),
        )
    }

    pub fn node_already_exists(path: impl Into<String>) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "NODE_ALREADY_EXISTS",
            format!("Node already exists at path: {}", path.into()),
        )
    }

    pub fn workspace_already_exists(workspace: impl Into<String>) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "WORKSPACE_ALREADY_EXISTS",
            format!("Workspace '{}' already exists", workspace.into()),
        )
    }

    // === Method Not Allowed (405) ===

    pub fn read_only_revision() -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "READ_ONLY_REVISION",
            "Cannot modify nodes at historic revision. Use HEAD for mutable operations.",
        )
    }

    // === Internal Server Errors (500) ===

    pub fn internal(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_SERVER_ERROR",
            &msg,
        )
        .with_details(msg)
    }

    pub fn storage_error(details: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORAGE_ERROR",
            "Storage operation failed",
        )
        .with_details(details)
    }

    pub fn serialization_error(details: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SERIALIZATION_ERROR",
            "Failed to serialize/deserialize data",
        )
        .with_details(details)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status;
        let body = Json(self);
        (status, body).into_response()
    }
}

// Convert from raisin_error::Error to ApiError
impl From<raisin_error::Error> for ApiError {
    fn from(err: raisin_error::Error) -> Self {
        match err {
            raisin_error::Error::NotFound(msg) => {
                // Try to extract entity type from message
                if msg.contains("Node") || msg.contains("node") {
                    ApiError::node_not_found(&msg).with_details(msg)
                } else if msg.contains("Branch") || msg.contains("branch") {
                    ApiError::branch_not_found(&msg).with_details(msg)
                } else if msg.contains("Tag") || msg.contains("tag") {
                    ApiError::tag_not_found(&msg).with_details(msg)
                } else if msg.contains("Repository") || msg.contains("repository") {
                    ApiError::repository_not_found(&msg).with_details(msg)
                } else if msg.contains("Workspace") || msg.contains("workspace") {
                    ApiError::workspace_not_found(&msg).with_details(msg)
                } else if msg.contains("NodeType") || msg.contains("node type") {
                    ApiError::node_type_not_found(&msg).with_details(msg)
                } else {
                    ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND", msg)
                }
            }
            raisin_error::Error::AlreadyExists(msg) => {
                // Entity already exists - return 409 Conflict
                if msg.contains("NodeType") {
                    ApiError::new(
                        StatusCode::CONFLICT,
                        "NODE_TYPE_ALREADY_EXISTS",
                        msg.clone(),
                    )
                    .with_details(msg)
                } else if msg.contains("Archetype") {
                    ApiError::new(
                        StatusCode::CONFLICT,
                        "ARCHETYPE_ALREADY_EXISTS",
                        msg.clone(),
                    )
                    .with_details(msg)
                } else if msg.contains("ElementType") {
                    ApiError::new(
                        StatusCode::CONFLICT,
                        "ELEMENT_TYPE_ALREADY_EXISTS",
                        msg.clone(),
                    )
                    .with_details(msg)
                } else {
                    ApiError::new(StatusCode::CONFLICT, "ALREADY_EXISTS", msg.clone())
                        .with_details(msg)
                }
            }
            raisin_error::Error::Validation(msg) => {
                ApiError::validation_failed(msg.clone()).with_details(msg)
            }
            raisin_error::Error::Conflict(msg) => {
                // Try to extract entity type from message
                if msg.contains("Branch") && msg.contains("exists") {
                    ApiError::branch_already_exists(&msg).with_details(msg)
                } else if msg.contains("Tag") && msg.contains("exists") {
                    ApiError::tag_already_exists(&msg).with_details(msg)
                } else if msg.contains("Repository") && msg.contains("exists") {
                    ApiError::repository_already_exists(&msg).with_details(msg)
                } else if msg.contains("Node") && msg.contains("exists") {
                    ApiError::node_already_exists(&msg).with_details(msg)
                } else if msg.contains("published") {
                    ApiError::node_already_published(&msg).with_details(msg)
                } else {
                    ApiError::new(StatusCode::CONFLICT, "CONFLICT", msg.clone()).with_details(msg)
                }
            }
            raisin_error::Error::Backend(msg) => {
                tracing::error!("Backend error: {}", msg);
                ApiError::storage_error(msg)
            }
            raisin_error::Error::Unauthorized(msg) => {
                ApiError::new(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", msg)
            }
            raisin_error::Error::Forbidden(msg) => {
                ApiError::new(StatusCode::FORBIDDEN, "FORBIDDEN", msg)
            }
            raisin_error::Error::PermissionDenied(msg) => {
                ApiError::new(StatusCode::FORBIDDEN, "PERMISSION_DENIED", msg)
            }
            raisin_error::Error::Lock(msg) => {
                tracing::error!("Lock error: {}", msg);
                ApiError::internal(format!("Lock error: {}", msg))
            }
            raisin_error::Error::Encoding(msg) => {
                tracing::error!("Encoding error: {}", msg);
                ApiError::new(StatusCode::BAD_REQUEST, "ENCODING_ERROR", msg)
            }
            raisin_error::Error::InvalidState(msg) => {
                tracing::error!("Invalid state: {}", msg);
                ApiError::internal(format!("Invalid state: {}", msg))
            }
            raisin_error::Error::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                ApiError::internal(msg)
            }
            raisin_error::Error::Other(e) => {
                tracing::error!("Unexpected error: {}", e);
                ApiError::internal(e.to_string())
            }
        }
    }
}

// Helper for converting serialization errors
impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        ApiError::invalid_json(err.to_string())
    }
}

impl From<serde_yaml::Error> for ApiError {
    fn from(err: serde_yaml::Error) -> Self {
        ApiError::serialization_error(err.to_string())
    }
}

#[cfg(feature = "storage-rocksdb")]
impl From<raisin_flow_runtime::types::FlowError> for ApiError {
    fn from(err: raisin_flow_runtime::types::FlowError) -> Self {
        use raisin_flow_runtime::types::FlowError;
        match err {
            FlowError::NodeNotFound(msg) => ApiError::not_found(msg),
            FlowError::InvalidDefinition(msg) => ApiError::validation_failed(msg),
            FlowError::InvalidStateTransition { from, to } => {
                ApiError::validation_failed(format!(
                    "Invalid state transition from {} to {}",
                    from, to
                ))
            }
            FlowError::AlreadyTerminated { status } => {
                ApiError::validation_failed(format!("Flow instance is already {}", status))
            }
            FlowError::NotSupported(msg) => ApiError::internal(msg),
            FlowError::Serialization(msg) => ApiError::internal(msg),
            other => ApiError::internal(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_serialization() {
        let err = ApiError::node_not_found("/test/path").with_details("Additional context");

        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("NODE_NOT_FOUND"));
        assert!(json.contains("Additional context"));
    }

    #[test]
    fn test_error_with_field() {
        let err = ApiError::missing_required_field("name");
        assert_eq!(err.field, Some("name".to_string()));
        assert_eq!(err.code, "MISSING_REQUIRED_FIELD");
    }

    #[test]
    fn test_from_raisin_error() {
        let raisin_err = raisin_error::Error::NotFound("Node not found".to_string());
        let api_err: ApiError = raisin_err.into();
        assert_eq!(api_err.code, "NODE_NOT_FOUND");
        assert_eq!(api_err.status, StatusCode::NOT_FOUND);
    }
}
