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

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

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

/// Refuse the request unless the caller may build or download a package
/// export (401 without a usable credential, 403 otherwise). `operator` is
/// whether the request carries an [`OperatorCredential`]
/// (crate::middleware::OperatorCredential): only then does a system context
/// count — an API key alone never exports.
///
/// The decision lives in `raisin_core::services::package_export_access`; every
/// export entry point (create-from-selection, `raisin:export`,
/// `raisin:download`) asks it BEFORE touching the job system or storage.
pub(crate) fn require_export_access(
    auth: Option<&AuthContext>,
    operator: bool,
) -> Result<(), ApiError> {
    use raisin_core::services::package_export_access::{authorize_export_for, ExportRefusal};
    authorize_export_for(auth, operator).map_err(|refusal| {
        let (status, code) = match refusal {
            ExportRefusal::Unauthenticated => {
                (axum::http::StatusCode::UNAUTHORIZED, "UNAUTHORIZED")
            }
            ExportRefusal::Forbidden => (axum::http::StatusCode::FORBIDDEN, "FORBIDDEN"),
        };
        ApiError::new(status, code, refusal.to_string())
    })
}

/// A signed-in caller (not the anonymous principal). Reads then run under the
/// caller's row-level security.
pub(crate) fn require_package_reader(auth: Option<AuthContext>) -> Result<AuthContext, ApiError> {
    match auth {
        Some(a) if !a.is_anonymous_principal() => Ok(a),
        _ => Err(ApiError::unauthorized(
            "package endpoints require signing in",
        )),
    }
}

/// A package administrator: the system context (the superadmin bearer, an
/// admin-console login, or an API key — the deploy tooling installs with
/// one, and a key already writes with system rights through the node API)
/// or an identity user holding `system_admin`. Anonymous is 401, anyone
/// else 403. Installing, uninstalling and uploading packages change schema
/// and content repository-wide, so nothing narrower qualifies.
pub(crate) fn require_package_admin(auth: Option<AuthContext>) -> Result<AuthContext, ApiError> {
    let a = require_package_reader(auth)?;
    if a.is_system || a.permissions().is_some_and(|p| p.is_system_admin) {
        Ok(a)
    } else {
        Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "managing packages requires an administrator",
        ))
    }
}

/// List all packages in the packages workspace.
///
/// GET /api/repos/{repo}/packages
pub async fn list_packages(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Vec<models::nodes::Node>>, ApiError> {
    let auth_context = Some(require_package_reader(auth.map(|Extension(ctx)| ctx))?);
    let tenant_id = tenant_info.tenant_id.as_str();
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
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<models::nodes::Node>, ApiError> {
    let auth_context = Some(require_package_reader(auth.map(|Extension(ctx)| ctx))?);
    let tenant_id = tenant_info.tenant_id.as_str();
    let branch = "main";
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, &repo, branch, workspace, auth_context);

    let node = node_service
        .get_by_path(&format!("/{}", package_name))
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_name)))?;

    Ok(Json(node))
}
