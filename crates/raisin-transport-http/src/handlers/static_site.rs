// SPDX-License-Identifier: BSL-1.1

//! Generic static-content endpoint: `GET /resources/{repo}/{branch}/{ws}/{*path}`.
//!
//! Serves a `raisin:Asset`/`raisin:Folder` subtree like a static file server,
//! reusing the same RLS-enforced, fail-closed read path as the repository
//! handlers ([`AppState::node_service_for_context`] → `NodeService`, never raw
//! storage). A folder (or a path resolving to one) serves its index document;
//! a `raisin:StaticSiteFolder` ancestor's `serving_config` drives the response
//! headers (CSP `frame-ancestors`, per-folder CORS, `Cache-Control`).
//!
//! Anonymous access falls through to whatever the anonymous role may read — a
//! folder is public precisely when `raisin:access_control` grants it, with no
//! separate auth mode.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Extension, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use raisin_binary::BinaryStorage;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

use crate::middleware::TenantInfo;
use crate::state::AppState;

const ASSET_TYPE: &str = "raisin:Asset";
const FOLDER_TYPE: &str = "raisin:Folder";
const STATIC_SITE_FOLDER_TYPE: &str = "raisin:StaticSiteFolder";
const DEFAULT_INDEX_DOCUMENT: &str = "index.html";

/// Per-folder serving overrides read off a `raisin:StaticSiteFolder`.
#[derive(Debug, Default, Clone, Deserialize)]
pub(crate) struct ServingConfig {
    #[serde(default)]
    frame_ancestors: Vec<String>,
    #[serde(default)]
    cors_allowed_origins: Vec<String>,
    #[serde(default)]
    cache_control: Option<String>,
    #[serde(default)]
    index_document: Option<String>,
}

/// One `raisin:StaticSiteFolder` in a workspace: its path and parsed config.
///
/// The full set for a workspace is cached (bounded by folder count) and scanned
/// in-memory to find the nearest folder covering a request path — so the cache
/// is keyed by workspace, never by the unbounded request path.
#[derive(Debug, Clone)]
pub(crate) struct StaticSiteEntry {
    folder_path: String,
    config: ServingConfig,
}

/// Serving decision for a request path.
///
/// A subtree is servable through `/resources` only when a
/// `raisin:StaticSiteFolder` ancestor covers it — the folder's *presence* is
/// the deny-by-default allowlist, shippable as ordinary package content. The
/// folder set is resolved with system auth (principal-independent) so it can be
/// safely shared across callers via [`AppState::static_site_cache`]; per-asset
/// authorization is still enforced separately by RLS when the bytes are served.
#[derive(Debug, Clone)]
enum ServingPolicy {
    /// No covering `raisin:StaticSiteFolder` — nothing here is servable.
    Denied,
    /// Covered by a `raisin:StaticSiteFolder`; carries the nearest folder's
    /// `serving_config` (defaulted when the folder declares none) for headers.
    Allowed(ServingConfig),
}

/// `GET /resources/{repo}/{branch}/{ws}/{*path}` — serve static content.
pub async fn serve_static(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path((repo, branch, ws, path)): Path<(String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    headers: HeaderMap,
) -> Response {
    let tenant_id = tenant_info.tenant_id.clone();
    let auth_ctx = auth.map(|Extension(ctx)| ctx);

    match resolve_and_serve(
        &state, &tenant_id, &repo, &branch, &ws, &path, auth_ctx, &headers,
    )
    .await
    {
        Ok(response) => response,
        Err(status) => status.into_response(),
    }
}

/// Resolve `{ws}/{path}` and serve either the asset bytes or a folder index.
#[allow(clippy::too_many_arguments)]
async fn resolve_and_serve(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    path: &str,
    auth: Option<AuthContext>,
    req_headers: &HeaderMap,
) -> Result<Response, StatusCode> {
    let trailing = path.is_empty() || path.ends_with('/');
    let node_path = normalize_node_path(path);

    // Deny-by-default serving gate: a path is servable only when a
    // `raisin:StaticSiteFolder` ancestor covers it. Resolved with system auth
    // (principal-independent) and cached, so this runs ahead of — and cheaper
    // than — the per-asset RLS read below. `Denied` and lookup errors alike
    // fail closed as 404 so folder/asset existence is never leaked.
    // Both gates below fail closed as 404 so existence is never leaked. That is
    // right for the response and useless for an operator: a missing folder, an
    // unreadable node and a genuinely absent file are indistinguishable from
    // outside AND produced no log line at all. Name which gate rejected, at
    // debug, so the answer costs one log level rather than a bisect.
    let config = match resolve_serving_policy(state, tenant_id, repo, branch, ws, &node_path).await
    {
        ServingPolicy::Allowed(config) => config,
        ServingPolicy::Denied => {
            tracing::debug!(
                tenant_id,
                repo,
                branch,
                ws,
                path = %node_path,
                "static serve DENIED: no raisin:StaticSiteFolder covers this path"
            );
            return Err(StatusCode::NOT_FOUND);
        }
    };

    let is_anonymous = auth.is_none();
    let svc = state.node_service_for_context(tenant_id, repo, branch, ws, auth);

    // RLS-scoped resolve: a node the caller may not see resolves to `None`.
    let node = svc
        .get_by_path(&node_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let node = match node {
        Some(node) => node,
        None => {
            // The serving policy already passed, so the subtree IS servable —
            // which makes an unresolvable node here far more likely to be an RLS
            // denial than a missing file. For an anonymous caller the usual
            // cause is anonymous access being disabled for the repo, which is
            // read from `/config/repos/{repo}` in `raisin:system`.
            tracing::debug!(
                tenant_id,
                repo,
                branch,
                ws,
                path = %node_path,
                is_anonymous,
                "static serve DENIED: node not visible to caller (missing, or hidden by RLS)"
            );
            return Err(StatusCode::NOT_FOUND);
        }
    };

    // A folder (or a trailing-slash request) serves its index document.
    if node.is_a(FOLDER_TYPE) || (trailing && !node.is_a(ASSET_TYPE)) {
        return serve_folder_index(state, &svc, &node, &config, req_headers).await;
    }

    if node.is_a(ASSET_TYPE) {
        return serve_asset(state, &node, false, &config, req_headers).await;
    }

    Err(StatusCode::NOT_FOUND)
}

/// Look up and serve a folder's index document.
async fn serve_folder_index(
    state: &AppState,
    svc: &raisin_core::NodeService<crate::state::Store>,
    folder: &Node,
    config: &ServingConfig,
    req_headers: &HeaderMap,
) -> Result<Response, StatusCode> {
    let index_document = config
        .index_document
        .clone()
        .unwrap_or_else(|| DEFAULT_INDEX_DOCUMENT.to_string());

    let base = folder.path.trim_end_matches('/');
    let index_path = format!("{base}/{index_document}");

    let index_node = svc
        .get_by_path(&index_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if !index_node.is_a(ASSET_TYPE) {
        return Err(StatusCode::NOT_FOUND);
    }

    serve_asset(state, &index_node, true, config, req_headers).await
}

/// Stream a `raisin:Asset`'s bytes with the resolved static-serving headers.
async fn serve_asset(
    state: &AppState,
    node: &Node,
    is_index: bool,
    config: &ServingConfig,
    req_headers: &HeaderMap,
) -> Result<Response, StatusCode> {
    let resource = match node.properties.get("file") {
        Some(PropertyValue::Resource(resource)) => resource,
        _ => return Err(StatusCode::NOT_FOUND),
    };

    let storage_key = resource
        .metadata
        .as_ref()
        .and_then(|m| m.get("storage_key"))
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or(StatusCode::NOT_FOUND)?;

    // `application/octet-stream` is upload-pipeline shorthand for "unknown" —
    // serving it verbatim makes browsers DOWNLOAD an html/css/js file instead
    // of rendering it, which breaks every iframe'd static site. Treat it like
    // an absent mime and re-guess from the filename.
    let mime_type = resource
        .mime_type
        .clone()
        .filter(|m| m != "application/octet-stream")
        .unwrap_or_else(|| {
            mime_guess::from_path(&node.path)
                .first_or_octet_stream()
                .to_string()
        });

    // ETag from the Asset's content hash; enables conditional 304 responses.
    // `content_hash` is a free-form string property a writer controls, so it may
    // not be a valid header value (e.g. it contains CR/LF). Validate it up front
    // and simply skip conditional-request handling when it isn't header-safe,
    // rather than panicking when the header is later inserted.
    let etag = node.properties.get("content_hash").and_then(|v| match v {
        PropertyValue::String(s) => Some(format!("\"{s}\"")),
        _ => None,
    });
    let etag_header = etag
        .as_deref()
        .and_then(|e| header::HeaderValue::from_str(e).ok());

    if let (Some(etag), Some(etag_header)) = (&etag, &etag_header) {
        if let Some(if_none_match) = req_headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
        {
            if if_none_match.split(',').any(|c| c.trim() == etag) {
                return Response::builder()
                    .status(StatusCode::NOT_MODIFIED)
                    .header(header::ETAG, etag_header.clone())
                    .body(Body::empty())
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    }

    // Stream the bytes rather than buffering the whole asset in memory — the
    // backend streams from disk (fs) or straight from the object body (S3), so a
    // large asset (video/PDF) costs constant memory regardless of size.
    let (content_length, stream) = state
        .bin
        .get_stream(&storage_key)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime_type)
        .header(header::CONTENT_LENGTH, content_length);

    if let Some(etag_header) = etag_header {
        builder = builder.header(header::ETAG, etag_header);
    }

    // Cache-Control: the index document defaults to no-cache (a stale SPA index
    // would keep serving an old route table); other assets are cacheable.
    let cache_control = config.cache_control.clone().unwrap_or_else(|| {
        if is_index {
            "no-cache".to_string()
        } else {
            "public, max-age=3600".to_string()
        }
    });
    builder = builder.header(header::CACHE_CONTROL, cache_control);

    // CSP frame-ancestors — only when explicitly configured (deny/no-header by
    // default; X-Frame-Options intentionally omitted so CSP governs framing).
    if !config.frame_ancestors.is_empty() {
        let value = format!("frame-ancestors {}", config.frame_ancestors.join(" "));
        builder = builder.header(header::CONTENT_SECURITY_POLICY, value);
    }

    // Per-folder CORS. Resolved here (not in middleware/cors.rs) so this
    // subtree-scoped policy cannot affect any other route; the global unified
    // CORS middleware still runs and is purely additive.
    if let Some(origin) = req_headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        // Never combine credential reflection with a wildcard: reflecting an
        // arbitrary Origin alongside `Allow-Credentials: true` lets any site
        // make credentialed cross-origin reads against RLS-gated content. Only
        // an explicitly-listed origin gets credentials; `*` is served without
        // them (browsers reject `*` + credentials anyway).
        let is_explicit = config.cors_allowed_origins.iter().any(|o| o == origin);
        let is_wildcard = config.cors_allowed_origins.iter().any(|o| o == "*");
        if is_explicit {
            builder = builder
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin)
                .header(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, "true")
                .header(header::VARY, "Origin");
        } else if is_wildcard {
            // Wildcard origin, intentionally no `Allow-Credentials`.
            builder = builder.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*");
        }
    }

    // Build fallibly: an Asset property that a writer poisoned into an invalid
    // header value (e.g. `mime_type` with CR/LF) yields a clean 500 here rather
    // than panicking the handler.
    builder
        .body(Body::from_stream(stream))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Normalize a wildcard-captured path into a workspace node path.
fn normalize_node_path(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("/{trimmed}")
    }
}

/// Resolve the serving policy for `path` against the workspace's set of
/// `raisin:StaticSiteFolder`s.
///
/// `path` is servable iff some `raisin:StaticSiteFolder` *covers* it (equals it
/// or is a `/`-boundary ancestor); the nearest (longest) such folder supplies
/// the `serving_config`. No cover ⇒ [`ServingPolicy::Denied`] (deny-by-default).
///
/// The folder **set** is enumerated with **system auth** (principal-independent,
/// so anonymous and authenticated callers get the same published/not-published
/// answer) and cached per `{tenant}\0{repo}\0{branch}\0{ws}` — a bounded key, so
/// unbounded/adversarial request paths can't grow the cache. The nearest-cover
/// match is then a pure in-memory scan over that small set. Same 60s TTL as the
/// CORS resolver; enumeration errors fail closed to `Denied` and are **not**
/// cached, so a transient failure can't poison the entry.
async fn resolve_serving_policy(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    path: &str,
) -> ServingPolicy {
    let key = format!("{tenant_id}\0{repo}\0{branch}\0{ws}");

    let entries = match state.static_site_cache.get(&key) {
        Some(cached) => cached,
        None => {
            let svc = state.node_service_for_context(
                tenant_id,
                repo,
                branch,
                ws,
                Some(AuthContext::system()),
            );
            let folders = match svc.list_by_type(STATIC_SITE_FOLDER_TYPE).await {
                Ok(folders) => folders,
                // Fail closed without caching, so a transient error isn't sticky.
                Err(_) => return ServingPolicy::Denied,
            };
            let entries: Arc<Vec<StaticSiteEntry>> = Arc::new(
                folders
                    .into_iter()
                    .map(|node| StaticSiteEntry {
                        config: parse_serving_config(&node).unwrap_or_default(),
                        folder_path: node.path,
                    })
                    .collect(),
            );
            state.static_site_cache.put(&key, entries.clone());
            entries
        }
    };

    match nearest_covering(&entries, path) {
        Some(entry) => ServingPolicy::Allowed(entry.config.clone()),
        None => ServingPolicy::Denied,
    }
}

/// The nearest `raisin:StaticSiteFolder` covering `path` — the longest folder
/// path that [`covers`] it — or `None` if none does.
fn nearest_covering<'a>(entries: &'a [StaticSiteEntry], path: &str) -> Option<&'a StaticSiteEntry> {
    entries
        .iter()
        .filter(|e| covers(&e.folder_path, path))
        .max_by_key(|e| e.folder_path.trim_end_matches('/').len())
}

/// Whether `folder` covers `path`: the same node, or a `/`-boundary ancestor.
///
/// The boundary check prevents `/site` from covering a sibling like `/site-x`.
fn covers(folder: &str, path: &str) -> bool {
    let folder = folder.trim_end_matches('/');
    let path = path.trim_end_matches('/');
    // An empty (workspace-root) folder path covers everything under it.
    folder.is_empty() || path == folder || path.starts_with(&format!("{folder}/"))
}

/// Parse a node's `serving_config` object property into [`ServingConfig`].
fn parse_serving_config(node: &Node) -> Option<ServingConfig> {
    let value = node.properties.get("serving_config")?;
    let json = serde_json::to_value(value).ok()?;
    serde_json::from_value(json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> StaticSiteEntry {
        StaticSiteEntry {
            folder_path: path.to_string(),
            config: ServingConfig::default(),
        }
    }

    #[test]
    fn covers_matches_self_and_boundary_descendants() {
        // The folder itself and anything genuinely under it.
        assert!(covers("/site", "/site"));
        assert!(covers("/site", "/site/assets/img/logo.png"));
        assert!(covers("/site/", "/site/app.js")); // trailing slash tolerated
    }

    #[test]
    fn covers_respects_path_boundaries() {
        // A sibling that merely shares a string prefix must NOT be covered —
        // otherwise `/site` would leak into `/site-private`.
        assert!(!covers("/site", "/site-private/secret.txt"));
        assert!(!covers("/site", "/other"));
    }

    #[test]
    fn nearest_covering_is_deny_by_default_and_picks_the_deepest() {
        // No StaticSiteFolder at all ⇒ nothing is servable (deny-by-default).
        assert!(nearest_covering(&[], "/site/app.js").is_none());
        assert!(nearest_covering(&[entry("/other")], "/site/app.js").is_none());

        // The nearest (longest) covering folder wins, so a deeper folder's
        // serving_config governs over a broader ancestor's.
        let entries = [entry("/site"), entry("/site/widgets/order")];
        let hit = nearest_covering(&entries, "/site/widgets/order/index.html").unwrap();
        assert_eq!(hit.folder_path, "/site/widgets/order");

        // A path only the broader folder covers falls back to it.
        let hit = nearest_covering(&entries, "/site/style.css").unwrap();
        assert_eq!(hit.folder_path, "/site");
    }

    #[test]
    fn normalize_node_path_trims_and_roots() {
        assert_eq!(normalize_node_path(""), "/");
        assert_eq!(normalize_node_path("/"), "/");
        assert_eq!(normalize_node_path("a/b"), "/a/b");
        assert_eq!(normalize_node_path("/a/b/"), "/a/b");
        assert_eq!(normalize_node_path("a/b/"), "/a/b");
    }
}
