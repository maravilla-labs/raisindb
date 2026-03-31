// SPDX-License-Identifier: BSL-1.1

//! Package management API handlers.
//!
//! Provides endpoints for managing RaisinDB packages (.rap files):
//! - Upload packages as ZIP archives
//! - Install/uninstall packages (extract node types, workspaces, content)
//! - Browse package contents without extracting
//! - List and manage installed packages
//! - Export, sync, and create packages from selections
//!
//! Packages are stored as `raisin:Package` nodes in the `packages` workspace.

mod browse;
mod commands;
mod export;
mod install;
mod sync;
pub mod types;
mod upload;

use axum::{
    extract::{Extension, Path, State},
    Json,
};
use raisin_models as models;
use raisin_models::auth::AuthContext;

use crate::{error::ApiError, state::AppState};

// Re-export public handler functions
pub use browse::{get_package_file, list_package_contents};
pub use commands::handle_package_command;
pub use export::export_package;
pub use install::{install_package, uninstall_package};
pub use sync::{create_package_from_selection, get_package_diff, get_sync_status};
pub use types::*;
pub use upload::extract_manifest;
#[allow(deprecated)]
pub use upload::upload_package;

/// List all packages in the packages workspace.
///
/// GET /api/repos/{repo}/packages
pub async fn list_packages(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Vec<models::nodes::Node>>, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let tenant_id = "default";
    let branch = "main";
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, &repo, branch, workspace, auth_context);

    let packages = node_service
        .list_by_type("raisin:Package")
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to list packages: {}", e)))?;

    Ok(Json(packages))
}

/// Get details for a specific package.
///
/// GET /api/repos/{repo}/packages/{name}
pub async fn get_package(
    State(state): State<AppState>,
    Path((repo, package_name)): Path<(String, String)>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<models::nodes::Node>, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let tenant_id = "default";
    let branch = "main";
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, &repo, branch, workspace, auth_context);
    let node_id = format!("package-{}", package_name);

    let node = node_service
        .get(&node_id)
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_name)))?;

    Ok(Json(node))
}
