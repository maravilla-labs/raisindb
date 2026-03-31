// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! External dependency management API endpoints.
//!
//! These endpoints allow the admin console to:
//! - View the status of external dependencies (Tesseract, etc.)
//! - Re-check and enable dependencies that were previously skipped

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Serialize;

/// Response for dependency list endpoint
#[derive(Debug, Serialize)]
pub struct DependenciesResponse {
    pub dependencies: Vec<crate::deps_setup::DependencyInfo>,
}

/// Response wrapper for API responses
#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(error: String) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(error),
        }
    }
}

/// State for dependency endpoints
#[derive(Clone)]
pub struct DepsState {
    pub data_dir: String,
}

/// GET /management/dependencies
///
/// Returns the status of all external dependencies.
pub async fn list_dependencies(
    State(state): State<DepsState>,
) -> Result<Json<ApiResponse<DependenciesResponse>>, StatusCode> {
    match crate::deps_setup::get_dependency_status(&state.data_dir) {
        Ok(deps) => Ok(Json(ApiResponse::ok(DependenciesResponse {
            dependencies: deps,
        }))),
        Err(e) => {
            tracing::error!(error = %e, "Failed to get dependency status");
            Ok(Json(ApiResponse::err(format!(
                "Failed to get dependency status: {}",
                e
            ))))
        }
    }
}

/// POST /management/dependencies/{name}/enable
///
/// Re-check a dependency and enable it if now available.
/// This is called when user clicks "Enable" button after installing the dependency.
pub async fn enable_dependency(
    State(state): State<DepsState>,
    Path(name): Path<String>,
) -> Result<Json<ApiResponse<crate::deps_setup::EnableResult>>, StatusCode> {
    tracing::info!(dependency = %name, "Attempting to enable dependency");

    match crate::deps_setup::try_enable_dependency(&state.data_dir, &name) {
        Ok(result) => {
            match &result {
                crate::deps_setup::EnableResult::Enabled { version } => {
                    tracing::info!(dependency = %name, version = %version, "Dependency enabled successfully");
                }
                crate::deps_setup::EnableResult::NotInstalled { .. } => {
                    tracing::warn!(dependency = %name, "Dependency still not installed");
                }
            }
            Ok(Json(ApiResponse::ok(result)))
        }
        Err(e) => {
            tracing::error!(dependency = %name, error = %e, "Failed to enable dependency");
            Ok(Json(ApiResponse::err(format!(
                "Failed to enable dependency: {}",
                e
            ))))
        }
    }
}
