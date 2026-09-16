// SPDX-License-Identifier: BSL-1.1

//! Asset binary access with signed URLs and scoped access grants.
//!
//! Provides functions for parsing asset commands from URL paths,
//! generating signed URLs for asset downloads/displays, minting scoped grants,
//! and serving asset content with credential validation.
//!
//! Two credential forms reach [`handle_asset_command_internal`], and they differ
//! in WHERE the authority lives:
//!
//! * `?sig=…&exp=…` — a per-asset signature. The signature IS the authority:
//!   [`sign_asset_url_internal`] checked access when it minted, so the read runs
//!   unfiltered. This is the machine-to-machine form and is unchanged.
//! * `?grant=…` — a scoped grant naming a SUBJECT. It is not authority; it says
//!   which subject and which subtree, and the read is then performed AS that
//!   subject under their row-level security. That is what makes a grant unable
//!   to return anything its subject could not fetch directly.
//!
//! Both go through one verifier, [`raisin_core::authorize_asset_read`], so the
//! two cannot drift into a 401 that no log line explains.

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

/// Parse the grant command from a path.
///
/// Returns the PREFIX the grant is being asked for if the path ends with
/// `/raisin:grant`. Unlike `raisin:sign`, what precedes it is a subtree rather
/// than one asset — `/photos/raisin:grant` asks to cover everything under
/// `/photos`.
pub(crate) fn parse_grant_command_from_path(path: &str) -> Option<String> {
    if let Some(idx) = path.rfind("/raisin:grant") {
        return Some(path[..idx].to_string());
    }
    None
}

/// Turn a refusal from the shared verifier into the HTTP answer for it.
///
/// The body never says more than the code: a caller learns that it must
/// re-authorize, not which part of its token was wrong.
///
/// A request that presented NO grant gets the answer it has always got —
/// `401 INVALID_SIGNATURE` — whatever the underlying reason. Existing clients
/// key on that code, and telling them apart "expired" from "wrong" is a
/// distinction the per-asset form never made and does not need: its remedy is
/// the same either way.
///
/// For a grant the codes are finer, because the client's retry contract depends
/// on them. 401 means "ask again" — the clock ran out, or this is not a
/// credential the deployment minted. 403 means the grant is valid but was minted
/// for a different subtree, which re-minting the SAME grant will not fix.
fn credential_error(err: raisin_core::AssetAuthError, presented_grant: bool) -> ApiError {
    if !presented_grant {
        return ApiError::new(
            StatusCode::UNAUTHORIZED,
            "INVALID_SIGNATURE",
            "Invalid or expired signature",
        );
    }

    let status = match err {
        raisin_core::AssetAuthError::OutOfScope => StatusCode::FORBIDDEN,
        _ => StatusCode::UNAUTHORIZED,
    };
    ApiError::new(status, err.code(), err.to_string())
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
    // The scoped grant, when the caller presented one instead of a signature.
    grant: Option<&str>,
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

    // ONE verifier for both credential forms. The path grammar and the HMAC live
    // in `raisin-core` with the minters that produce them; a verifier that
    // spelled either itself is how a minter and a verifier drift into a
    // permanent 401 that no log line can explain.
    let signing_secret = state.get_signing_secret()?;
    let scope = raisin_core::AssetReadScope {
        tenant_id,
        repo,
        branch,
        workspace: ws,
        node_path: &node_path,
        property: prop_name,
        command,
    };
    let presented_grant = grant.is_some_and(|g| !g.is_empty());
    let credential = raisin_core::AssetCredential::from_query(Some(sig), Some(exp), grant)
        .map_err(|e| credential_error(e, presented_grant))?;
    let authorization = raisin_core::authorize_asset_read(&signing_secret, scope, credential)
        .map_err(|e| credential_error(e, presented_grant))?;

    // WHERE the node is read from is the security difference between the two
    // forms, and it is the only difference.
    //
    // A signature was minted for this one asset after its minter checked access,
    // so the read is unfiltered — that is the historical behaviour and the
    // machine-to-machine contract.
    //
    // A grant is not authority. It names a subject, and the read runs as that
    // subject with row-level security applied, resolved NOW. So the grant can
    // never return a node its subject could not fetch directly, and withdrawing
    // the subject's access stops the grant working without anything having to
    // revoke the token itself.
    let node = match &authorization {
        raisin_core::AssetAuthorization::Signature => state
            .storage()
            .nodes()
            .get_by_path(
                StorageScope::new(tenant_id, repo, branch, ws),
                &node_path,
                None,
            )
            .await?
            .ok_or_else(|| ApiError::not_found("Node not found"))?,
        raisin_core::AssetAuthorization::Grant(grant) => {
            read_as_grant_subject(state, tenant_id, repo, branch, ws, &node_path, grant).await?
        }
    };

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

/// Read a node AS a grant's subject, with row-level security applied.
///
/// The subject's permissions are resolved HERE rather than carried in the token.
/// That costs a lookup on the read path — cached, like every other request's —
/// and buys the property the grant rests on: what the grant opens is exactly
/// what the subject may read at the moment of the read, so access withdrawn
/// mid-session closes the grant with it.
///
/// `email` and `home` come from the token because they are identity claims the
/// subject's own session asserted, and row-level security conditions may
/// reference them. They were copied from the minting principal's authenticated
/// context, never from the mint request.
///
/// A node the subject may not read is reported as missing, not as forbidden: the
/// existence of a node at a path is itself something row-level security is
/// entitled to hide.
async fn read_as_grant_subject(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    node_path: &str,
    grant: &raisin_core::AssetGrant,
) -> Result<raisin_models::nodes::Node, ApiError> {
    use raisin_core::PermissionService;
    use raisin_models::auth::AuthContext;

    let permissions = state
        .permission_service
        .resolve_for_identity_id(tenant_id, repo, "main", &grant.subject)
        .await
        .map_err(|e| {
            tracing::error!(
                subject = %grant.subject,
                error = %e,
                "Could not resolve permissions for an asset grant's subject"
            );
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "PERMISSION_RESOLUTION_FAILED",
                "Could not resolve access for this grant",
            )
        })?;

    // Built exactly as the auth middleware builds a live session's context, so
    // the grant read and the session read evaluate the same conditions. A
    // subject with no permissions in this repository gets a context with none —
    // which denies, because row-level security denies by default.
    let mut auth = AuthContext::for_user(&grant.subject);
    if let Some(email) = &grant.email {
        auth = auth.with_email(email);
    }
    if let Some(home) = &grant.home {
        auth = auth.with_home(home);
    }
    if let Some(permissions) = permissions {
        auth = auth.with_permissions(permissions);
    }

    state
        .node_service_for_context(tenant_id, repo, branch, ws, Some(auth))
        .get_by_path(node_path)
        .await?
        .ok_or_else(|| ApiError::not_found("Node not found"))
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

/// Request body for minting a scoped asset grant.
#[derive(Debug, Default, serde::Deserialize)]
pub struct GrantAssetRequest {
    /// Requested lifetime in seconds. Clamped to
    /// [`raisin_core::MAX_GRANT_LIFETIME_SECS`]; a grant is meant to be renewed,
    /// not to be long.
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/// Response carrying a minted grant and the scope it covers.
#[derive(Debug, serde::Serialize)]
pub struct GrantAssetResponse {
    /// The opaque token to append to asset URLs as `?grant=…`.
    pub grant: String,
    /// The normalized path prefix the grant covers.
    pub prefix: String,
    /// When the grant expires (ISO 8601). The client renews before or on 401.
    pub expires_at: String,
    /// Unix seconds, for a client that would rather not parse a date.
    pub expires: u64,
}

/// Mint a scoped asset grant for the authenticated caller.
///
/// The grant's SUBJECT is the caller, taken from their authenticated context and
/// never from the request body. That is the property that makes a grant
/// incapable of escalation: a caller can only ask for a token that says "read as
/// me", and every read it authorizes is then performed under the caller's own
/// row-level security.
///
/// Two principals are refused, both deliberately:
///
/// * **Anonymous.** A grant bound to nobody is a plain bearer token for whatever
///   anonymous may read, which public asset delivery already covers better.
/// * **System / admin.** Their context bypasses row-level security, so a grant
///   naming them would be a standing key to a subtree with no filtering behind
///   it — exactly the thing the design refuses to mint. These callers already
///   have per-asset signing, which is the right instrument for them.
pub(crate) async fn mint_asset_grant_internal(
    state: &AppState,
    auth: Option<&raisin_models::auth::AuthContext>,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    prefix: &str,
    request: GrantAssetRequest,
) -> Result<Json<GrantAssetResponse>, ApiError> {
    let auth = auth.ok_or_else(|| {
        ApiError::unauthorized("A scoped asset grant requires an authenticated session")
    })?;

    if auth.is_anonymous || auth.is_system {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "GRANT_REQUIRES_USER",
            "A scoped asset grant must name a user; anonymous and system callers use per-asset signing",
        ));
    }

    let subject = auth.user_id.clone().ok_or_else(|| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "GRANT_REQUIRES_USER",
            "A scoped asset grant must name a user; this credential names none",
        )
    })?;

    // Normalized once, here, and stored normalized — so the token carries the
    // one spelling the segment matcher will later compare against.
    let prefix = raisin_core::normalize_path(prefix)
        .ok_or_else(|| ApiError::validation_failed("Grant prefix is not a valid node path"))?;

    // The prefix must be something the caller can actually see. Every read the
    // grant authorizes is filtered again anyway, so this is not what makes the
    // grant safe — it is what stops a caller probing for the existence of
    // subtrees by minting grants over them.
    if prefix != "/" {
        state
            .node_service_for_context(tenant_id, repo, branch, ws, Some(auth.clone()))
            .get_by_path(&prefix)
            .await?
            .ok_or_else(|| ApiError::not_found("Node not found"))?;
    }

    let lifetime = raisin_core::clamp_grant_lifetime(
        request
            .expires_in
            .unwrap_or(raisin_core::DEFAULT_GRANT_LIFETIME_SECS),
    );
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + lifetime;

    let grant = raisin_core::AssetGrant {
        tenant_id: tenant_id.to_string(),
        repo: repo.to_string(),
        branch: branch.to_string(),
        workspace: ws.to_string(),
        prefix: prefix.clone(),
        subject,
        email: auth.email.clone(),
        home: auth.home.clone(),
        expires,
    };

    let signing_secret = state.get_signing_secret()?;
    let token = raisin_core::mint_asset_grant(&signing_secret, &grant);

    let expires_at = chrono::DateTime::from_timestamp(expires as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "unknown".to_string());

    Ok(Json(GrantAssetResponse {
        grant: token,
        prefix,
        expires_at,
        expires,
    }))
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
