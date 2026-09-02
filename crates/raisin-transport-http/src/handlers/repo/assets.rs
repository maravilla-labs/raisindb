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
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    path: &str,
    command: &str,
    property_path: Option<&str>,
    sig: &str,
    exp: u64,
    // The raw `Range` header, threaded down from the request. Without it a
    // `<video>` served from here has a dead scrub bar — see `http_range`.
    range_header: Option<&str>,
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

    // The signed string comes from the SHARED composer, never rebuilt here: a
    // verifier that spells the grammar itself is how a minter and a verifier
    // drift into a permanent 401 that no log line can explain.
    let full_path = raisin_core::signed_asset_path(repo, branch, ws, &node_path, prop_name);

    // Verify signature - include property_path in verification
    let signing_secret = state.get_signing_secret()?;
    if !raisin_core::verify_asset_signature(
        &signing_secret,
        tenant_id,
        &full_path,
        command,
        raisin_core::signature_property(prop_name),
        exp,
        sig,
    ) {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "INVALID_SIGNATURE",
            "Invalid or expired signature",
        ));
    }

    // Get node (tenant comes from request context — signature already validated access)
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

    // A mounted file whose bytes are not held right now is NOT a missing
    // property — it is a cache miss on a file that still exists at the provider.
    //
    // A mount syncs metadata only, and a cached copy expires once nothing needs
    // it, so this is the ordinary steady state for a synced drive rather than an
    // error. Filling it here is what makes reading a mounted asset work exactly
    // like reading a local one: same URL, same caller, no new API. Doing it in
    // the client instead would mean every reader — the console, an SDK, a
    // browser following a signed link — had to know that virtual mounts exist.
    //
    // Only for the file itself: a missing `thumbnail` is a derived artifact that
    // was never made, and no fetch can conjure it.
    let node = if prop_name == "file" && !node.properties.contains_key(prop_name) {
        hydrate_mounted_asset(&state, tenant_id, repo, branch, ws, &node)
            .await
            .unwrap_or(node)
    } else {
        node
    };

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

    // Range resolution happens HERE, at the end, against the bytes we actually
    // hold — after the signature check and after the mount hydration above. A
    // mounted asset whose cache had expired has been fetched by this point, so
    // a range request over one is served like any other rather than refused,
    // and the total below is the real entity size either way.
    //
    // The bytes are already fully in memory (`BinaryStorage::get` has no ranged
    // read), so slicing is all a partial response can be today. `Bytes::slice`
    // is a refcount bump, not a copy — the win here is protocol correctness,
    // seeking, not reduced IO. A ranged storage read would be the follow-up.
    let total = bytes.len() as u64;
    let resolution = super::http_range::resolve(range_header, total);

    // `Accept-Ranges` goes on EVERY outcome, including the plain 200. A browser
    // decides whether seeking is possible from this header on its first probe;
    // omit it there and the scrub bar stays dead even though ranges work.
    let base = Response::builder()
        .header(header::CONTENT_TYPE, mime_type)
        .header(header::CONTENT_DISPOSITION, disposition)
        .header(header::CACHE_CONTROL, "private, max-age=300")
        .header(header::ACCEPT_RANGES, "bytes");

    let response = match resolution {
        // A well-formed request for bytes that do not exist. 416 carries
        // `bytes */total` so the client learns the real size and can re-ask,
        // rather than being told 200 and keeping its wrong belief.
        super::http_range::RangeResolution::Unsatisfiable => base
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{}", total))
            .header(header::CONTENT_LENGTH, 0)
            .body(Body::empty()),
        super::http_range::RangeResolution::Satisfiable { start, end } => {
            // Content-Length is the length of the PART, not of the file.
            // Sending the whole size here makes the browser wait forever for
            // bytes that are never coming.
            let part = bytes.slice(start as usize..=end as usize);
            let len = end - start + 1;
            base.status(StatusCode::PARTIAL_CONTENT)
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {}-{}/{}", start, end, total),
                )
                .header(header::CONTENT_LENGTH, len)
                .body(Body::from(part))
        }
        super::http_range::RangeResolution::None => base
            .status(StatusCode::OK)
            .header(header::CONTENT_LENGTH, total)
            .body(Body::from(bytes)),
    };

    Ok(response.expect("valid response with valid headers"))
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
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    path: &str,
    request: SignAssetRequest,
) -> Result<Json<SignAssetResponse>, ApiError> {
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
            StorageScope::new(tenant_id, repo, branch, ws),
            &node_path,
            None,
        )
        .await?
        .ok_or_else(|| ApiError::not_found("Node not found"))?;

    // Validate node has the requested property.
    //
    // A mount-owned asset whose cache is empty has NO `file` property, and that
    // is a cache state rather than an absence: `serve_asset` fetches the bytes
    // from the provider when the signed URL is read. Signing needs the node, the
    // property name and RLS — never the bytes — so refusing here would reject
    // the very request that fills the cache. Only `file` is admitted this way,
    // because that is the property the fetch writes.
    let missing_but_fetchable = property_path == "file"
        && !node.properties.contains_key(property_path)
        && raisin_models::nodes::is_fetchable_mount_content(&node.properties);

    if !missing_but_fetchable {
        let file_prop = node.properties.get(property_path).ok_or_else(|| {
            ApiError::validation_failed(format!(
                "Node does not have a '{}' property",
                property_path
            ))
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
    }

    // Generate expiry timestamp
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + request.expires_in;

    // One composer for the path grammar, the signature and the URL — shared
    // with the serve handler that verifies and with the
    // `raisin.assets.signedUrl` function binding that mints for a media
    // service. A second spelling of any of the three is an unexplainable 401.
    //
    // This surface keeps its historical behaviour of falling back to a
    // ROOT-RELATIVE URL when no base is configured: its consumer is a browser
    // that already has an origin. The function binding refuses instead, because
    // its consumer is another process.
    let signing_secret = state.get_signing_secret()?;
    let base_url = raisin_core::configured_public_base_url();
    let signed = raisin_core::build_signed_asset_url(
        &signing_secret,
        tenant_id,
        repo,
        branch,
        ws,
        &node_path,
        property_path,
        &request.command,
        expires,
        base_url.as_deref(),
    );
    let url = signed.url;

    let expires_at = chrono::DateTime::from_timestamp(expires as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "unknown".to_string());

    Ok(Json(SignAssetResponse { url, expires_at }))
}

/// Fetch a mount-owned asset's bytes on demand, and return the node that now
/// carries them.
///
/// `None` when there was nothing to do or nothing could be done — the caller
/// keeps the node it had and reports the ordinary "no such property", because a
/// provider being unreachable should read as a missing file rather than as a
/// server fault.
///
/// The fetch also re-stamps `__content_cached_at`, so READING a file extends its
/// lease: something being looked at is the best evidence it should stay warm.
#[cfg(feature = "storage-rocksdb")]
async fn hydrate_mounted_asset(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    node: &raisin_models::nodes::Node,
) -> Option<raisin_models::nodes::Node> {
    if !raisin_models::nodes::is_fetchable_mount_content(&node.properties) {
        return None;
    }

    let rocksdb = state.rocksdb_storage()?;
    let mounts = rocksdb.virtual_mount_sync_handler()?;

    // The mount's config lives on the repo's config branch, which is NOT the
    // branch the asset was materialized on. Passing one for the other resolves
    // no mount at all.
    let config_branch = crate::handlers::integrations::config_branch(state, tenant_id, repo).await;

    let fetched = mounts
        .fetch_content(
            raisin_rocksdb::ContentTarget {
                tenant: tenant_id,
                repo,
                config_branch: &config_branch,
                branch,
                workspace: ws,
                node_id: &node.id,
            },
            false,
        )
        .await;

    if let Err(e) = fetched {
        tracing::warn!(
            node_id = %node.id, error = %e,
            "Could not fetch mounted content for a signed asset read"
        );
        return None;
    }

    // Re-read: the fetch wrote the `file` onto the stored node.
    state
        .storage()
        .nodes()
        .get_by_path(
            StorageScope::new(tenant_id, repo, branch, ws),
            &node.path,
            None,
        )
        .await
        .ok()
        .flatten()
}
