// SPDX-License-Identifier: BSL-1.1

//! Package browsing and file retrieval handlers.
//!
//! Browse contents of `.rap` ZIP archives and retrieve individual files,
//! supporting nested `.rap` files up to a maximum nesting depth.

use axum::{
    extract::{Extension, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use raisin_binary::BinaryStorage;
use raisin_models as models;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use std::io::{Cursor, Read};
use zip::ZipArchive;

use crate::{error::ApiError, state::AppState};

use super::types::{FileType, PackageFile, ZipContentsResponse, ZipEntry};

/// Maximum nesting depth for .rap files within .rap files
const MAX_NESTING_DEPTH: usize = 3;

/// List contents of a package ZIP without extracting.
///
/// GET /api/repos/{repo}/packages/{name}/contents
pub async fn list_package_contents(
    State(state): State<AppState>,
    Path((repo, package_name)): Path<(String, String)>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<ZipContentsResponse>, ApiError> {
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

    let resource_key = extract_resource_key(&node)?;
    let zip_data = state.bin.get(resource_key).await.map_err(|e| {
        ApiError::storage_error(format!("Failed to retrieve package binary: {}", e))
    })?;

    let cursor = Cursor::new(zip_data);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| ApiError::validation_failed(format!("Invalid ZIP file: {}", e)))?;

    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| ApiError::storage_error(format!("Failed to read ZIP entry: {}", e)))?;

        entries.push(ZipEntry {
            path: file.name().to_string(),
            size: file.size(),
            compressed_size: file.compressed_size(),
            is_dir: file.is_dir(),
        });
    }

    let total = entries.len();
    Ok(Json(ZipContentsResponse { entries, total }))
}

/// Get a specific file from the package ZIP.
///
/// GET /api/repos/{repo}/packages/{name}/contents/{*path}
pub async fn get_package_file(
    State(state): State<AppState>,
    Path((repo, package_name, file_path)): Path<(String, String, String)>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Response, ApiError> {
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

    let resource_key = extract_resource_key(&node)?;
    let zip_data = state.bin.get(resource_key).await.map_err(|e| {
        ApiError::storage_error(format!("Failed to retrieve package binary: {}", e))
    })?;

    let cursor = Cursor::new(zip_data);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| ApiError::validation_failed(format!("Invalid ZIP file: {}", e)))?;

    let mut file = archive
        .by_name(&file_path)
        .map_err(|_| ApiError::not_found(format!("File '{}' not found in package", file_path)))?;

    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|e| ApiError::storage_error(format!("Failed to read file: {}", e)))?;

    let content_type = mime_guess::from_path(&file_path)
        .first_or_octet_stream()
        .to_string();

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        contents,
    )
        .into_response())
}

/// Browse package contents at a specific path within the ZIP.
pub(super) async fn browse_package_impl(
    state: &AppState,
    repo: &str,
    branch: &str,
    package_path: &str,
    zip_path: &str,
    auth_context: Option<AuthContext>,
) -> Result<Vec<PackageFile>, ApiError> {
    let tenant_id = "default";
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, repo, branch, workspace, auth_context);

    let node = node_service
        .get_by_path(&format!("/{}", package_path))
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_path)))?;

    let resource_key = extract_resource_key(&node)?;
    let zip_data = state.bin.get(resource_key).await.map_err(|e| {
        ApiError::storage_error(format!("Failed to retrieve package binary: {}", e))
    })?;

    let zip_vec = zip_data.to_vec();
    let cursor = Cursor::new(zip_vec);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| ApiError::validation_failed(format!("Invalid ZIP file: {}", e)))?;

    browse_archive_at_path(&mut archive, zip_path, 1)
}

/// Get a specific file from the package ZIP (command endpoint).
pub(super) async fn get_file_from_package_impl(
    state: &AppState,
    repo: &str,
    branch: &str,
    package_path: &str,
    zip_path: &str,
    auth_context: Option<AuthContext>,
) -> Result<Response, ApiError> {
    let tenant_id = "default";
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, repo, branch, workspace, auth_context);

    let node = node_service
        .get_by_path(&format!("/{}", package_path))
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_path)))?;

    let resource_key = extract_resource_key(&node)?;
    let zip_data = state.bin.get(resource_key).await.map_err(|e| {
        ApiError::storage_error(format!("Failed to retrieve package binary: {}", e))
    })?;

    let zip_vec = zip_data.to_vec();
    let cursor = Cursor::new(zip_vec);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| ApiError::validation_failed(format!("Invalid ZIP file: {}", e)))?;

    let contents = get_file_from_archive_at_path(&mut archive, zip_path, 1)?;

    let final_filename = zip_path.rsplit('/').next().unwrap_or(zip_path);
    let content_type = mime_guess::from_path(final_filename)
        .first_or_octet_stream()
        .to_string();

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        contents,
    )
        .into_response())
}

// --- Archive helpers ---

/// Extract the resource key from a package node.
fn extract_resource_key(node: &models::nodes::Node) -> Result<&str, ApiError> {
    let resource = node
        .properties
        .get("resource")
        .ok_or_else(|| ApiError::validation_failed("Package has no resource"))?;

    let resource_obj = match resource {
        PropertyValue::Object(obj) => obj,
        _ => {
            return Err(ApiError::validation_failed(
                "Resource is not a valid object",
            ))
        }
    };

    resource_obj
        .get("key")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.as_str()),
            _ => None,
        })
        .ok_or_else(|| ApiError::validation_failed("Resource has no key"))
}

/// Split path at first .rap boundary for nested package browsing.
fn split_at_rap_boundary(path: &str) -> Option<(&str, &str)> {
    if let Some(pos) = path.find(".rap/") {
        let rap_end = pos + 4;
        let outer_path = &path[..rap_end];
        let inner_path = &path[rap_end + 1..];
        Some((outer_path, inner_path))
    } else {
        None
    }
}

/// Read a file from a ZIP archive and return its contents.
fn read_file_from_archive(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    path: &str,
) -> Result<Vec<u8>, ApiError> {
    let mut file = archive
        .by_name(path)
        .map_err(|_| ApiError::not_found(format!("File '{}' not found in archive", path)))?;

    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|e| ApiError::storage_error(format!("Failed to read file: {}", e)))?;

    Ok(contents)
}

/// Recursively browse archive contents, supporting nested .rap files.
fn browse_archive_at_path(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    path: &str,
    depth: usize,
) -> Result<Vec<PackageFile>, ApiError> {
    if depth > MAX_NESTING_DEPTH {
        return Err(ApiError::validation_failed(format!(
            "Maximum nesting depth ({}) exceeded",
            MAX_NESTING_DEPTH
        )));
    }

    if let Some((outer_path, inner_path)) = split_at_rap_boundary(path) {
        let nested_data = read_file_from_archive(archive, outer_path)?;
        let cursor = Cursor::new(nested_data);
        let mut nested_archive = ZipArchive::new(cursor).map_err(|e| {
            ApiError::validation_failed(format!("Invalid nested ZIP file '{}': {}", outer_path, e))
        })?;
        return browse_archive_at_path(&mut nested_archive, inner_path, depth + 1);
    }

    list_directory_entries(archive, path)
}

/// List directory entries at a given path in an archive.
fn list_directory_entries(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    path: &str,
) -> Result<Vec<PackageFile>, ApiError> {
    let normalized_path = path.trim_matches('/');
    let prefix = if normalized_path.is_empty() {
        String::new()
    } else {
        format!("{}/", normalized_path)
    };

    let mut entries_map: std::collections::HashMap<String, PackageFile> =
        std::collections::HashMap::new();

    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| ApiError::storage_error(format!("Failed to read ZIP entry: {}", e)))?;

        let file_name = file.name().to_string();

        if !prefix.is_empty() && !file_name.starts_with(&prefix) {
            continue;
        }
        if file_name.trim_end_matches('/') == normalized_path {
            continue;
        }

        let relative_path = if prefix.is_empty() {
            &file_name
        } else {
            &file_name[prefix.len()..]
        };

        if relative_path.is_empty() {
            continue;
        }

        let first_component = relative_path.split('/').next().unwrap_or("");
        if first_component.is_empty() {
            continue;
        }

        let is_directory = relative_path.contains('/') || file.is_dir();

        let entry_key = first_component.to_string();
        if !entries_map.contains_key(&entry_key) {
            let full_path = if prefix.is_empty() {
                first_component.to_string()
            } else {
                format!("{}{}", prefix, first_component)
            };

            entries_map.insert(
                entry_key.clone(),
                PackageFile {
                    path: full_path,
                    name: first_component.to_string(),
                    file_type: if is_directory {
                        FileType::Directory
                    } else {
                        FileType::File
                    },
                    size: if is_directory {
                        None
                    } else {
                        Some(file.size())
                    },
                    mime_type: if is_directory {
                        None
                    } else {
                        Some(
                            mime_guess::from_path(first_component)
                                .first_or_octet_stream()
                                .to_string(),
                        )
                    },
                },
            );
        }
    }

    let mut entries: Vec<PackageFile> = entries_map.into_values().collect();
    entries.sort_by(|a, b| match (&a.file_type, &b.file_type) {
        (FileType::Directory, FileType::File) => std::cmp::Ordering::Less,
        (FileType::File, FileType::Directory) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });

    Ok(entries)
}

/// Recursively get a file from archive, supporting nested .rap files.
fn get_file_from_archive_at_path(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    path: &str,
    depth: usize,
) -> Result<Vec<u8>, ApiError> {
    if depth > MAX_NESTING_DEPTH {
        return Err(ApiError::validation_failed(format!(
            "Maximum nesting depth ({}) exceeded",
            MAX_NESTING_DEPTH
        )));
    }

    if let Some((outer_path, inner_path)) = split_at_rap_boundary(path) {
        let nested_data = read_file_from_archive(archive, outer_path)?;
        let cursor = Cursor::new(nested_data);
        let mut nested_archive = ZipArchive::new(cursor).map_err(|e| {
            ApiError::validation_failed(format!("Invalid nested ZIP file '{}': {}", outer_path, e))
        })?;
        return get_file_from_archive_at_path(&mut nested_archive, inner_path, depth + 1);
    }

    read_file_from_archive(archive, path)
}
