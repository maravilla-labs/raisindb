// SPDX-License-Identifier: BSL-1.1

//! Asset binary access with signed URLs.
//!
//! Provides functions for parsing asset commands from URL paths,
//! generating signed URLs for asset downloads/displays, and
//! serving asset content with signature validation.

use axum::{
    body::Body,
    extract::Json,
    http::{header, StatusCode},
    response::Response,
};
use raisin_binary::BinaryStorage;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use crate::{error::ApiError, middleware::RaisinContext, state::AppState};

/// Parse asset command from a path.
/// Returns (asset_path, command) if path ends with /raisin:download or /raisin:display.
pub(crate) fn parse_asset_command_from_path(path: &str) -> Option<(String, String)> {
    for cmd in &["raisin:download", "raisin:display"] {
        if let Some(idx) = path.rfind(&format!("/{}", cmd)) {
            let asset_path = path[..idx].to_string();
            let command = cmd.replace("raisin:", "");
            return Some((asset_path, command));
        }
    }
    None
}

/// Parse sign command from a path.
/// Returns the asset path if path ends with /raisin:sign.
pub(crate) fn parse_sign_command_from_path(path: &str) -> Option<String> {
    if let Some(idx) = path.rfind("/raisin:sign") {
        return Some(path[..idx].to_string());
    }
    None
}

/// Internal implementation of asset command handling.
pub(crate) async fn handle_asset_command_internal(
    state: &AppState,
    repo: &str,
    branch: &str,
    ws: &str,
    path: &str,
    command: &str,
    property_path: Option<&str>,
    sig: &str,
    exp: u64,
) -> Result<Response, ApiError> {
    // Validate command
    if command != "download" && command != "display" {
        return Err(ApiError::validation_failed(
            "command must be 'download' or 'display'",
        ));
    }

    // Normalize path
    let node_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    // Get property name - default to "file" if not specified
    let prop_name = property_path.unwrap_or("file");

    // Create the full path for signature verification
    // Include @property_path suffix if not "file" (for backward compatibility)
    let full_path = if prop_name == "file" {
        format!("{}/{}/head/{}{}", repo, branch, ws, node_path)
    } else {
        format!("{}/{}/head/{}{}@{}", repo, branch, ws, node_path, prop_name)
    };

    // Verify signature - include property_path in verification
    let signing_secret = state.get_signing_secret()?;
    let prop_option = if prop_name == "file" {
        None
    } else {
        property_path
    };
    if !raisin_core::verify_asset_signature(
        &signing_secret,
        &full_path,
        command,
        prop_option,
        exp,
        sig,
    ) {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "INVALID_SIGNATURE",
            "Invalid or expired signature",
        ));
    }

    // Use default tenant for signed URL access (signature already validated access)
    let tenant_id = "default";

    // Get node
    let node = state
        .storage()
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo, branch, ws),
            &node_path,
            None,
        )
        .await?
        .ok_or_else(|| ApiError::not_found("Node not found"))?;

    // Get the requested property
    let file_prop = node.properties.get(prop_name).ok_or_else(|| {
        ApiError::not_found(format!("Node does not have a '{}' property", prop_name))
    })?;

    // Extract resource
    let resource = match file_prop {
        raisin_models::nodes::properties::PropertyValue::Resource(r) => r,
        _ => {
            return Err(ApiError::not_found(format!(
                "Node's '{}' property is not a Resource type",
                prop_name
            )));
        }
    };

    // Handle external resources
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

    // For internal storage, get storage_key from metadata
    let storage_key = resource
        .metadata
        .as_ref()
        .and_then(|m| m.get("storage_key"))
        .and_then(|v| match v {
            raisin_models::nodes::properties::PropertyValue::String(s) => Some(s.clone()),
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

    // Get filename
    let filename = resource
        .name
        .as_deref()
        .or_else(|| {
            std::path::Path::new(&storage_key)
                .file_name()
                .and_then(|n| n.to_str())
        })
        .unwrap_or("download");

    // Get MIME type
    let mime_type = resource.mime_type.clone().unwrap_or_else(|| {
        mime_guess::from_path(filename)
            .first_or_octet_stream()
            .to_string()
    });

    // Set Content-Disposition based on command
    let disposition = match command {
        "download" => format!("attachment; filename=\"{}\"", filename),
        "display" => "inline".to_string(),
        _ => "attachment".to_string(),
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime_type)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CACHE_CONTROL, "private, max-age=300")
        .body(Body::from(bytes))
        .expect("valid response with valid headers"))
}

/// Request body for signing an asset URL
#[derive(Debug, serde::Deserialize)]
pub struct SignAssetRequest {
    /// Command type: "download" or "display"
    pub command: String,
    /// Expiry time in seconds (default: 300)
    #[serde(default = "default_expires_in")]
    pub expires_in: u64,
}

fn default_expires_in() -> u64 {
    300
}

/// Response containing the signed URL
#[derive(Debug, serde::Serialize)]
pub struct SignAssetResponse {
    /// The signed URL for accessing the asset
    pub url: String,
    /// When the URL expires (ISO 8601)
    pub expires_at: String,
}

/// Internal implementation of sign URL generation.
pub(crate) async fn sign_asset_url_internal(
    state: &AppState,
    ctx: &RaisinContext,
    repo: &str,
    branch: &str,
    ws: &str,
    path: &str,
    request: SignAssetRequest,
) -> Result<Json<SignAssetResponse>, ApiError> {
    let _tenant_id = "default"; // TODO: Extract from auth context

    // Validate command
    if request.command != "download" && request.command != "display" {
        return Err(ApiError::validation_failed(
            "command must be 'download' or 'display'",
        ));
    }

    // Normalize path
    let node_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    // Get property path from context (extracted from @notation in URL)
    // Default to "file" if not specified
    let property_path = ctx.property_path.as_deref().unwrap_or("file");

    // Get node to validate it exists and user has access
    let node = state
        .storage()
        .nodes()
        .get_by_path(
            StorageScope::new("default", repo, branch, ws),
            &node_path,
            None,
        )
        .await?
        .ok_or_else(|| ApiError::not_found("Node not found"))?;

    // Validate node has the requested property
    let file_prop = node.properties.get(property_path).ok_or_else(|| {
        ApiError::validation_failed(format!("Node does not have a '{}' property", property_path))
    })?;

    // Validate it's a Resource type
    match file_prop {
        raisin_models::nodes::properties::PropertyValue::Resource(_) => {}
        _ => {
            return Err(ApiError::validation_failed(format!(
                "Node's '{}' property is not a Resource type",
                property_path
            )));
        }
    }

    // Generate expiry timestamp
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + request.expires_in;

    // Create the path to sign (includes full context)
    // Include @property_path suffix only if not "file" (for backward compatibility)
    let full_path = if property_path == "file" {
        format!("{}/{}/head/{}{}", repo, branch, ws, node_path)
    } else {
        format!(
            "{}/{}/head/{}{}@{}",
            repo, branch, ws, node_path, property_path
        )
    };

    // Sign the URL - include property_path in signature for security
    let signing_secret = state.get_signing_secret()?;
    let prop_option = if property_path == "file" {
        None
    } else {
        Some(property_path)
    };
    let signature = raisin_core::sign_asset_url(
        &signing_secret,
        &full_path,
        &request.command,
        prop_option,
        expires,
    );

    // Get base URL from environment or use relative path
    let base_url = std::env::var("RAISINDB_BASE_URL").unwrap_or_default();

    let url = format!(
        "{}/api/repository/{}/raisin:{}?sig={}&exp={}",
        base_url, full_path, request.command, signature, expires
    );

    let expires_at = chrono::DateTime::from_timestamp(expires as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "unknown".to_string());

    Ok(Json(SignAssetResponse { url, expires_at }))
}
