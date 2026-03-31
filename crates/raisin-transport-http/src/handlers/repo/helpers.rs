// SPDX-License-Identifier: BSL-1.1

//! Helper functions for repository handlers.
//!
//! Includes node version operations, property access, and file download handlers.

use axum::{
    body::Body,
    extract::Json,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use raisin_models::auth::AuthContext;

use raisin_binary::BinaryStorage;

use crate::{error::ApiError, state::AppState};

/// Get a specific version of a node
pub(crate) async fn get_node_version(
    state: &AppState,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    workspace: &str,
    path: &str,
    version_id: i32,
    auth: Option<AuthContext>,
) -> Result<Response, ApiError> {
    let nodes_svc = state.node_service_for_context(tenant_id, repository, branch, workspace, auth);
    let version = nodes_svc
        .get_version(path, version_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "Version {} not found for path {}",
                version_id, path
            ))
        })?;
    Ok(Json(version).into_response())
}

/// List all versions of a node
pub(crate) async fn list_node_versions(
    state: &AppState,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    workspace: &str,
    path: &str,
    auth: Option<AuthContext>,
) -> Result<Response, ApiError> {
    let nodes_svc = state.node_service_for_context(tenant_id, repository, branch, workspace, auth);
    let versions = nodes_svc.list_versions(path).await?;
    Ok(Json(serde_json::json!({
        "versions": versions
    }))
    .into_response())
}

/// Get a specific property from a node
///
/// Auto-detects Resource and String types to stream content inline:
/// - Resource (internal): Streams binary content with Content-Type header
/// - Resource (external): 307 redirect to URL
/// - String: Returns as text with guessed MIME type if path has file extension
/// - Other types: Returns JSON
pub(crate) async fn get_property(
    state: &AppState,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    workspace: &str,
    path: &str,
    property_path: &str,
    auth: Option<AuthContext>,
) -> Result<Response, ApiError> {
    use raisin_models::nodes::properties::PropertyValue;

    let nodes_svc = state.node_service_for_context(tenant_id, repository, branch, workspace, auth);
    let value = nodes_svc
        .get_property_by_path(path, property_path)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "Property '{}' not found at path {}",
                property_path, path
            ))
        })?;

    // Auto-detect Resource types and stream content inline
    match &value {
        PropertyValue::Resource(resource) => {
            // Handle external resources (redirect to URL)
            if resource.is_external == Some(true) {
                if let Some(url) = &resource.url {
                    return Ok(Response::builder()
                        .status(StatusCode::TEMPORARY_REDIRECT)
                        .header(header::LOCATION, url)
                        .body(Body::empty())
                        .expect("valid redirect response"));
                }
                // External resource without URL - fall through to JSON
            } else {
                // For internal storage, stream the content inline
                let storage_key = resource
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("storage_key"))
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    });

                if let Some(key) = storage_key {
                    // Fetch from binary storage
                    let bytes = state.bin.get(&key).await.map_err(|e| {
                        ApiError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "STORAGE_ERROR",
                            format!("Failed to retrieve file: {}", e),
                        )
                    })?;

                    let mime_type = resource.mime_type.clone().unwrap_or_else(|| {
                        let filename = resource.name.as_deref().unwrap_or("file");
                        mime_guess::from_path(filename)
                            .first_or_octet_stream()
                            .to_string()
                    });

                    // Stream inline (no Content-Disposition: attachment)
                    return Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, mime_type)
                        .body(Body::from(bytes))
                        .expect("valid response with valid headers"));
                }
                // No storage key - fall through to JSON
            }
        }
        PropertyValue::String(content) => {
            // For string content, check if it looks like file content
            // by checking if the property path ends with common file extensions
            let looks_like_file = property_path.ends_with(".file")
                || property_path == "file"
                || property_path.contains("file_content")
                || property_path.contains("content");

            if looks_like_file {
                // Guess MIME type from the node path
                let mime_type = mime_guess::from_path(path)
                    .first_or_text_plain()
                    .to_string();

                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime_type)
                    .body(Body::from(content.clone()))
                    .expect("valid response"));
            }
            // Not a file-like property - fall through to JSON
        }
        _ => {
            // Other types: return as JSON
        }
    }

    Ok(Json(value).into_response())
}

/// Handle file download - returns file content with appropriate headers
pub(crate) async fn handle_file_download(
    state: &AppState,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    workspace: &str,
    path: &str,
    property_path: Option<&str>,
    auth: Option<AuthContext>,
) -> Result<Response, ApiError> {
    use raisin_models::nodes::properties::PropertyValue;

    let nodes_svc = state.node_service_for_context(tenant_id, repository, branch, workspace, auth);

    // Get the property value (either from specific property or default "file" property)
    let property_value = if let Some(prop_path) = property_path {
        nodes_svc
            .get_property_by_path(path, prop_path)
            .await?
            .ok_or_else(|| {
                ApiError::not_found(format!(
                    "Property '{}' not found at path '{}'",
                    prop_path, path
                ))
            })?
    } else {
        // Get node and look for "file" property
        let node = nodes_svc
            .get_by_path(path)
            .await?
            .ok_or_else(|| ApiError::node_not_found(path))?;

        node.properties.get("file").cloned().ok_or_else(|| {
            ApiError::not_found(format!("No 'file' property found at path '{}'", path))
        })?
    };

    match property_value {
        PropertyValue::String(content) => {
            // Inline file content
            let filename = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("inline-file.txt");

            let archetype = mime_guess::from_path(filename)
                .first_or_octet_stream()
                .to_string();

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, archetype)
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", filename),
                )
                .body(Body::from(content))
                .expect("valid response with valid headers"))
        }
        PropertyValue::Resource(resource) => {
            // Handle external resources (just redirect to URL)
            if resource.is_external == Some(true) {
                if let Some(url) = &resource.url {
                    return Ok(Response::builder()
                        .status(StatusCode::TEMPORARY_REDIRECT)
                        .header(header::LOCATION, url)
                        .body(Body::empty())
                        .expect("valid redirect response"));
                } else {
                    return Err(ApiError::not_found("External resource has no URL"));
                }
            }

            // For internal storage, we need the storage_key from metadata
            let storage_key = resource
                .metadata
                .as_ref()
                .and_then(|m| m.get("storage_key"))
                .and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .ok_or_else(|| ApiError::not_found("Resource has no storage_key in metadata"))?;

            // Fetch from binary storage
            let bytes = state.bin.get(&storage_key).await.map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "STORAGE_ERROR",
                    format!("Failed to retrieve file: {}", e),
                )
            })?;

            let filename = resource
                .name
                .as_deref()
                .or_else(|| {
                    std::path::Path::new(&storage_key)
                        .file_name()
                        .and_then(|n| n.to_str())
                })
                .unwrap_or("download");

            let archetype = resource.mime_type.clone().unwrap_or_else(|| {
                mime_guess::from_path(filename)
                    .first_or_octet_stream()
                    .to_string()
            });

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, archetype)
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", filename),
                )
                .body(Body::from(bytes))
                .expect("valid response with valid headers"))
        }
        _ => Err(ApiError::validation_failed(
            "Property must be a String or Resource type for file download",
        )),
    }
}
