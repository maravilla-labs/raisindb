// SPDX-License-Identifier: BSL-1.1

//! Package command dispatch handler.
//!
//! Parses `raisin:` commands from URL paths and routes them
//! to the appropriate handler (browse, file, install, export, etc.).

use axum::{
    extract::{Extension, Path, State},
    response::{IntoResponse, Response},
    Json,
};
use raisin_models::auth::AuthContext;

use crate::{error::ApiError, state::AppState};

use super::types::{ExportPackageRequest, InstallMode, InstallQuery, PackageCommand};

/// Handle package commands (browse, file, install, export, download).
///
/// GET/POST /api/packages/{repo}/{branch}/head/{*path}
///
/// The path contains the package path followed by a raisin: command:
/// - ai-tools-1.0.0/raisin:browse -> browse root
/// - ai-tools-1.0.0/raisin:browse/nodetypes -> browse subdir
/// - ai-tools-1.0.0/raisin:file/manifest.yaml -> get file
/// - ai-tools-1.0.0/raisin:install -> install package (POST)
/// - ai-tools-1.0.0/raisin:export -> export package (POST)
/// - ai-tools-1.0.0/raisin:download/job-id -> download exported package (GET)
/// - ai-tools-1.0.0/raisin:sync-status -> sync status (GET)
pub async fn handle_package_command(
    State(state): State<AppState>,
    Path((repo, branch, full_path)): Path<(String, String, String)>,
    axum::extract::Query(install_query): axum::extract::Query<InstallQuery>,
    auth: Option<Extension<AuthContext>>,
    method: axum::http::Method,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);

    tracing::debug!(
        auth_present = auth_context.is_some(),
        auth_is_system = auth_context.as_ref().map(|a| a.is_system).unwrap_or(false),
        repo = %repo,
        path = %full_path,
        "handle_package_command: auth context status"
    );

    let (package_path, command) = parse_package_command(&full_path)?;

    match command {
        PackageCommand::Browse { zip_path } => {
            let result = super::browse::browse_package_impl(
                &state,
                &repo,
                &branch,
                &package_path,
                &zip_path,
                auth_context,
            )
            .await?;
            Ok(Json(result).into_response())
        }
        PackageCommand::File { zip_path } => {
            super::browse::get_file_from_package_impl(
                &state,
                &repo,
                &branch,
                &package_path,
                &zip_path,
                auth_context,
            )
            .await
        }
        PackageCommand::Install => {
            if method != axum::http::Method::POST {
                return Err(ApiError::validation_failed(
                    "raisin:install requires POST method",
                ));
            }
            let result = super::install::install_package_impl(
                &state,
                &repo,
                &branch,
                &package_path,
                install_query.mode,
                auth_context,
            )
            .await?;
            Ok(Json(result).into_response())
        }
        PackageCommand::DryRun { .. } => {
            let result = super::install::dry_run_impl(
                &state,
                &repo,
                &branch,
                &package_path,
                install_query.mode,
                auth_context,
            )
            .await?;
            Ok(Json(result).into_response())
        }
        PackageCommand::Export => {
            if method != axum::http::Method::POST {
                return Err(ApiError::validation_failed(
                    "raisin:export requires POST method",
                ));
            }
            let request: ExportPackageRequest = if body.is_empty() {
                ExportPackageRequest {
                    export_mode: "all".to_string(),
                    include_modifications: true,
                }
            } else {
                serde_json::from_slice(&body).map_err(|e| {
                    ApiError::validation_failed(format!("Invalid request body: {}", e))
                })?
            };
            let result = super::export::export_package_impl(
                &state,
                &repo,
                &branch,
                &package_path,
                request,
                auth_context,
            )
            .await?;
            Ok(Json(result).into_response())
        }
        PackageCommand::Download { job_id } => {
            if method != axum::http::Method::GET {
                return Err(ApiError::validation_failed(
                    "raisin:download requires GET method",
                ));
            }
            super::export::download_exported_package_impl(
                &state,
                &repo,
                &branch,
                &package_path,
                &job_id,
                auth_context,
            )
            .await
        }
        PackageCommand::SyncStatus => {
            if method != axum::http::Method::GET {
                return Err(ApiError::validation_failed(
                    "raisin:sync-status requires GET method",
                ));
            }
            let result = super::sync::get_sync_status_impl(
                &state,
                &repo,
                &branch,
                &package_path,
                auth_context,
            )
            .await?;
            Ok(Json(result).into_response())
        }
    }
}

/// Parse the full path to extract package path and command.
fn parse_package_command(full_path: &str) -> Result<(String, PackageCommand), ApiError> {
    let commands = [
        "/raisin:browse",
        "/raisin:file",
        "/raisin:install",
        "/raisin:dry-run",
        "/raisin:export",
        "/raisin:download",
        "/raisin:sync-status",
    ];

    for cmd in commands {
        if let Some(pos) = full_path.find(cmd) {
            let package_path = &full_path[..pos];
            let remainder = &full_path[pos + cmd.len()..];
            let path_arg = remainder.trim_start_matches('/').to_string();

            let command = match cmd {
                "/raisin:browse" => PackageCommand::Browse { zip_path: path_arg },
                "/raisin:file" => {
                    if path_arg.is_empty() {
                        return Err(ApiError::validation_failed(
                            "raisin:file requires a file path",
                        ));
                    }
                    PackageCommand::File { zip_path: path_arg }
                }
                "/raisin:install" => PackageCommand::Install,
                "/raisin:dry-run" => PackageCommand::DryRun {
                    mode: InstallMode::default(),
                },
                "/raisin:export" => PackageCommand::Export,
                "/raisin:download" => {
                    if path_arg.is_empty() {
                        return Err(ApiError::validation_failed(
                            "raisin:download requires a job ID",
                        ));
                    }
                    PackageCommand::Download { job_id: path_arg }
                }
                "/raisin:sync-status" => PackageCommand::SyncStatus,
                _ => unreachable!(),
            };

            return Ok((package_path.to_string(), command));
        }
    }

    Err(ApiError::validation_failed(
        "Path must contain raisin:browse, raisin:file, raisin:install, raisin:dry-run, raisin:export, raisin:download, or raisin:sync-status command"
    ))
}
