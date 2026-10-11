// SPDX-License-Identifier: BSL-1.1

//! Package browsing and file retrieval handlers.
//!
//! Browse contents of `.rap` ZIP archives and retrieve individual files,
//! supporting nested `.rap` files up to a maximum nesting depth.

use axum::body::Bytes;
use axum::{
    body::Body,
    extract::{Extension, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures::StreamExt;
use raisin_binary::zip_stream::{self, CentralEntry};
use raisin_binary::{BinaryStorage, ByteReadStream};
use raisin_models as models;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use std::io::{Cursor, Read};
use zip::ZipArchive;

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

use super::types::{FileType, PackageFile, ZipContentsResponse, ZipEntry};

/// Maximum nesting depth for .rap files within .rap files
const MAX_NESTING_DEPTH: usize = 3;

/// A listing reads the whole central directory; this bounds what it may hold.
const MAX_LISTED_CENTRAL_DIRECTORY: u64 = 64 * 1024 * 1024;

/// A nested `.rap` is opened in memory; this bounds how large one may be.
const MAX_NESTED_ARCHIVE: u64 = 256 * 1024 * 1024;

/// List contents of a package ZIP without extracting.
///
/// GET /api/repos/{repo}/packages/{name}/contents
pub async fn list_package_contents(
    State(state): State<AppState>,
    Path((repo, package_name)): Path<(String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<ZipContentsResponse>, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let tenant_id = tenant_info.tenant_id.as_str();
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
    // The central directory alone, by ranged reads: no entry body is read.
    let entries: Vec<ZipEntry> = outer_entries(&state, resource_key)
        .await?
        .into_iter()
        .map(|e| ZipEntry {
            is_dir: e.is_dir(),
            path: e.name,
            size: e.size,
            compressed_size: e.compressed_size,
        })
        .collect();

    let total = entries.len();
    Ok(Json(ZipContentsResponse { entries, total }))
}

/// Get a specific file from the package ZIP.
///
/// GET /api/repos/{repo}/packages/{name}/contents/{*path}
pub async fn get_package_file(
    State(state): State<AppState>,
    Path((repo, package_name, file_path)): Path<(String, String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Response, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let tenant_id = tenant_info.tenant_id.as_str();
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
    serve_file_at_path(&state, resource_key, &file_path).await
}

/// Browse package contents at a specific path within the ZIP.
pub(super) async fn browse_package_impl(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    package_path: &str,
    zip_path: &str,
    auth_context: Option<AuthContext>,
) -> Result<Vec<PackageFile>, ApiError> {
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, repo, branch, workspace, auth_context);

    let node = node_service
        .get_by_path(&format!("/{}", package_path))
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_path)))?;

    let resource_key = extract_resource_key(&node)?;
    if let Some((outer_path, inner_path)) = split_at_rap_boundary(zip_path) {
        let mut nested = open_nested(state, resource_key, outer_path).await?;
        return browse_archive_at_path(&mut nested, inner_path, 2);
    }
    let entries = outer_entries(state, resource_key).await?;
    Ok(list_directory_entries(
        entries
            .iter()
            .map(|e| (e.name.as_str(), e.size, e.is_dir())),
        zip_path,
    ))
}

/// Get a specific file from the package ZIP (command endpoint).
pub(super) async fn get_file_from_package_impl(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    package_path: &str,
    zip_path: &str,
    auth_context: Option<AuthContext>,
) -> Result<Response, ApiError> {
    let workspace = "packages";

    let node_service =
        state.node_service_for_context(tenant_id, repo, branch, workspace, auth_context);

    let node = node_service
        .get_by_path(&format!("/{}", package_path))
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to get package node: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_path)))?;

    let resource_key = extract_resource_key(&node)?;
    serve_file_at_path(state, resource_key, zip_path).await
}

/// Serve one file of a stored archive. A top-level entry is streamed from
/// a ranged read of its body (inflated on the fly); only an entry of a nested
/// `.rap` needs that nested archive in memory.
async fn serve_file_at_path(
    state: &AppState,
    resource_key: &str,
    zip_path: &str,
) -> Result<Response, ApiError> {
    let final_filename = zip_path.rsplit('/').next().unwrap_or(zip_path);
    let content_type = mime_guess::from_path(final_filename)
        .first_or_octet_stream()
        .to_string();

    if let Some((outer_path, inner_path)) = split_at_rap_boundary(zip_path) {
        let mut nested = open_nested(state, resource_key, outer_path).await?;
        let contents = get_file_from_archive_at_path(&mut nested, inner_path, 2)?;
        return Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, content_type)],
            contents,
        )
            .into_response());
    }

    let len = archive_len(state, resource_key).await?;
    let entry = zip_stream::find_entry(len, |r| fetch_range(state, resource_key, r), zip_path)
        .await
        .map_err(|e| ApiError::validation_failed(format!("Invalid ZIP file: {e}")))?
        .ok_or_else(|| ApiError::not_found(format!("File '{}' not found in package", zip_path)))?;
    let body = entry_stream(state, resource_key, &entry).await?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, entry.size)
        .body(Body::from_stream(body))
        .map_err(|e| ApiError::storage_error(format!("Failed to build response: {e}")))
}

// --- Ranged archive access ---

fn storage_error(e: anyhow::Error) -> ApiError {
    ApiError::storage_error(format!("Failed to read package archive: {e}"))
}

async fn archive_len(state: &AppState, key: &str) -> Result<u64, ApiError> {
    state
        .bin
        .head(key)
        .await
        .map_err(storage_error)?
        .map(|m| m.size)
        .ok_or_else(|| ApiError::not_found("Package archive not found in storage"))
}

/// One ranged read, collected (callers only ask for small ranges).
async fn fetch_range(
    state: &AppState,
    key: &str,
    r: std::ops::Range<u64>,
) -> anyhow::Result<Bytes> {
    let mut stream = state.bin.get_range(key, r).await?;
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk?);
    }
    Ok(Bytes::from(out))
}

async fn outer_entries(state: &AppState, key: &str) -> Result<Vec<CentralEntry>, ApiError> {
    let len = archive_len(state, key).await?;
    zip_stream::read_central_directory(
        len,
        |r| fetch_range(state, key, r),
        MAX_LISTED_CENTRAL_DIRECTORY,
    )
    .await
    .map_err(|e| ApiError::validation_failed(format!("Invalid ZIP file: {e}")))
}

/// The (uncompressed) body of one top-level entry, streamed.
async fn entry_stream(
    state: &AppState,
    key: &str,
    entry: &CentralEntry,
) -> Result<ByteReadStream, ApiError> {
    let range = zip_stream::entry_data_range(entry, |r| fetch_range(state, key, r))
        .await
        .map_err(|e| ApiError::validation_failed(format!("Invalid ZIP file: {e}")))?;
    let raw = state
        .bin
        .get_range(key, range)
        .await
        .map_err(storage_error)?;
    Ok(if entry.is_deflated() {
        zip_stream::inflate_stream(raw)
    } else {
        raw
    })
}

/// A nested `.rap`, read into memory (bounded by [`MAX_NESTED_ARCHIVE`]).
async fn open_nested(
    state: &AppState,
    key: &str,
    outer_path: &str,
) -> Result<ZipArchive<Cursor<Vec<u8>>>, ApiError> {
    let len = archive_len(state, key).await?;
    let entry = zip_stream::find_entry(len, |r| fetch_range(state, key, r), outer_path)
        .await
        .map_err(|e| ApiError::validation_failed(format!("Invalid ZIP file: {e}")))?
        .ok_or_else(|| {
            ApiError::not_found(format!("File '{}' not found in archive", outer_path))
        })?;
    if entry.size > MAX_NESTED_ARCHIVE {
        return Err(ApiError::validation_failed(format!(
            "Nested archive '{}' is larger than {} bytes and cannot be browsed",
            outer_path, MAX_NESTED_ARCHIVE
        )));
    }
    let mut body = entry_stream(state, key, &entry).await?;
    let mut data = Vec::with_capacity(entry.size as usize);
    while let Some(chunk) = body.next().await {
        data.extend_from_slice(&chunk.map_err(|e| storage_error(e.into()))?);
    }
    ZipArchive::new(Cursor::new(data)).map_err(|e| {
        ApiError::validation_failed(format!("Invalid nested ZIP file '{}': {}", outer_path, e))
    })
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

    let mut names = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| ApiError::storage_error(format!("Failed to read ZIP entry: {}", e)))?;
        names.push((file.name().to_string(), file.size(), file.is_dir()));
    }
    Ok(list_directory_entries(
        names.iter().map(|(n, s, d)| (n.as_str(), *s, *d)),
        path,
    ))
}

/// List directory entries at a given path in an archive.
fn list_directory_entries<'a>(
    files: impl Iterator<Item = (&'a str, u64, bool)>,
    path: &str,
) -> Vec<PackageFile> {
    let normalized_path = path.trim_matches('/');
    let prefix = if normalized_path.is_empty() {
        String::new()
    } else {
        format!("{}/", normalized_path)
    };

    let mut entries_map: std::collections::HashMap<String, PackageFile> =
        std::collections::HashMap::new();

    for (file_name, file_size, file_is_dir) in files {
        if !prefix.is_empty() && !file_name.starts_with(&prefix) {
            continue;
        }
        if file_name.trim_end_matches('/') == normalized_path {
            continue;
        }

        let relative_path = if prefix.is_empty() {
            file_name
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

        let is_directory = relative_path.contains('/') || file_is_dir;

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
                    size: if is_directory { None } else { Some(file_size) },
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

    entries
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
