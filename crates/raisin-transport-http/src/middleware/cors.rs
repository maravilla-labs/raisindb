// SPDX-License-Identifier: BSL-1.1

//! CORS middleware layers.
//!
//! Provides hierarchical CORS configuration resolution (repo > tenant > global)
//! and middleware for applying CORS headers to HTTP responses.
//!
//! For routes without a repo in the URL (e.g. `/api/uploads`), CORS origins are
//! aggregated across all repos for the tenant and cached with a short TTL.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};

use crate::state::AppState;

use super::path_helpers::{extract_repo_from_any_path, extract_repo_from_auth_path};
use super::types::TenantInfo;

/// Per-repository CORS middleware for auth endpoints.
///
/// Applies CORS headers based on the repository's auth configuration.
/// Handles `/auth/{repo}/*` routes by loading `RepoAuthConfig` for
/// `cors_allowed_origins`.
///
/// For OPTIONS preflight requests, returns 204 No Content with CORS headers.
#[cfg(feature = "storage-rocksdb")]
pub async fn repo_auth_cors_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    use axum::http::header;

    let path = req.uri().path();

    let repo_id = match extract_repo_from_auth_path(path) {
        Some(repo) => repo,
        None => return Ok(next.run(req).await),
    };

    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let tenant_id = req
        .extensions()
        .get::<TenantInfo>()
        .map(|t| t.tenant_id.clone())
        .unwrap_or_else(|| "default".to_string());

    let allowed_origins =
        get_cors_allowed_origins_for_repo(state.storage(), &tenant_id, &repo_id).await;

    let is_origin_allowed = match (&origin, &allowed_origins) {
        (Some(origin), origins) if !origins.is_empty() => origins
            .iter()
            .any(|allowed| allowed == origin || allowed == "*"),
        _ => false,
    };

    // Handle preflight OPTIONS request
    if req.method() == axum::http::Method::OPTIONS {
        let mut response = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .expect("empty 204 response is valid");

        if is_origin_allowed {
            if let Some(origin) = &origin {
                apply_preflight_cors_headers(response.headers_mut(), origin);
            }
        }

        return Ok(response);
    }

    let mut response = next.run(req).await;

    if is_origin_allowed {
        if let Some(origin) = &origin {
            apply_response_cors_headers(response.headers_mut(), origin);
        }
    }

    Ok(response)
}

/// Unified CORS middleware for all routes.
///
/// Applies CORS headers based on hierarchical configuration:
/// 1. Repository-level (RepoAuthConfig node) - highest priority
///    1b. Aggregate all repos for tenant (when no repo in URL) - e.g. `/api/uploads`
///    2. Tenant-level (TenantAuthConfig) - middle priority
///    3. Global (TOML config) - fallback
///
/// Results are cached with a 60s TTL in `AppState.cors_cache`.
///
/// For OPTIONS preflight requests, returns 204 No Content with CORS headers.
#[cfg(feature = "storage-rocksdb")]
pub async fn unified_cors_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    use axum::http::header;

    let path = req.uri().path();
    let method = req.method().clone();

    let tenant_id = req
        .extensions()
        .get::<TenantInfo>()
        .map(|t| t.tenant_id.clone())
        .unwrap_or_else(|| "default".to_string());

    let repo_id = extract_repo_from_any_path(path);

    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // No origin header means same-origin request, no CORS needed
    let Some(origin) = origin else {
        return Ok(next.run(req).await);
    };

    let allowed_origins = resolve_cors_allowed_origins(
        state.storage(),
        &tenant_id,
        repo_id.as_deref(),
        &state.cors_allowed_origins,
        &state.cors_cache,
    )
    .await;

    let is_origin_allowed = if allowed_origins.is_empty() {
        false
    } else {
        allowed_origins
            .iter()
            .any(|allowed| allowed == &origin || allowed == "*")
    };

    // Handle preflight OPTIONS request
    if method == axum::http::Method::OPTIONS {
        let mut response = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .expect("empty 204 response is valid");

        if is_origin_allowed {
            let headers = response.headers_mut();
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                origin
                    .parse()
                    .unwrap_or_else(|_| "*".parse().expect("hardcoded '*' is valid")),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                "GET, POST, PUT, DELETE, OPTIONS, PATCH"
                    .parse()
                    .expect("hardcoded methods are valid"),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                "Content-Type, Authorization, Accept, Content-Range, X-Tenant-Id, X-Raisin-Impersonate, Cache-Control"
                    .parse()
                    .expect("hardcoded headers are valid"),
            );
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                "true".parse().expect("hardcoded 'true' is valid"),
            );
            headers.insert(
                header::ACCESS_CONTROL_MAX_AGE,
                "86400".parse().expect("hardcoded '86400' is valid"),
            );
        }

        return Ok(response);
    }

    let mut response = next.run(req).await;

    if is_origin_allowed {
        let headers = response.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            origin
                .parse()
                .unwrap_or_else(|_| "*".parse().expect("hardcoded '*' is valid")),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            "true".parse().expect("hardcoded 'true' is valid"),
        );
        headers.insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            "Content-Range, Accept-Ranges"
                .parse()
                .expect("hardcoded headers are valid"),
        );
    }

    Ok(response)
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Apply CORS headers for preflight (OPTIONS) responses.
#[cfg(feature = "storage-rocksdb")]
fn apply_preflight_cors_headers(headers: &mut axum::http::HeaderMap, origin: &str) {
    use axum::http::header;

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        origin
            .parse()
            .unwrap_or_else(|_| "*".parse().expect("hardcoded '*' is valid")),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        "GET, POST, PUT, DELETE, OPTIONS, PATCH"
            .parse()
            .expect("hardcoded methods are valid"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        "Content-Type, Authorization, Accept, Cache-Control"
            .parse()
            .expect("hardcoded headers are valid"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        "true".parse().expect("hardcoded 'true' is valid"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        "86400".parse().expect("hardcoded '86400' is valid"),
    );
}

/// Apply CORS headers for regular (non-preflight) responses.
#[cfg(feature = "storage-rocksdb")]
fn apply_response_cors_headers(headers: &mut axum::http::HeaderMap, origin: &str) {
    use axum::http::header;

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        origin
            .parse()
            .unwrap_or_else(|_| "*".parse().expect("hardcoded '*' is valid")),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        "true".parse().expect("hardcoded 'true' is valid"),
    );
}

/// Get CORS allowed origins from RepoAuthConfig stored in system workspace.
#[cfg(feature = "storage-rocksdb")]
async fn get_cors_allowed_origins_for_repo(
    storage: &std::sync::Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
) -> Vec<String> {
    use raisin_core::services::node_service::NodeService;
    use raisin_models::auth::AuthContext;
    use raisin_models::nodes::properties::PropertyValue;

    let repo_config_path = format!("/config/repos/{}", repo_id);

    let node_service: NodeService<raisin_rocksdb::RocksDBStorage> = NodeService::new_with_context(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        "main".to_string(),
        "raisin:system".to_string(),
    )
    .with_auth(AuthContext::system());

    let repo_config_node = node_service
        .get_by_path(&repo_config_path)
        .await
        .ok()
        .flatten();

    if let Some(node) = repo_config_node {
        if node.node_type == "raisin:RepoAuthConfig" {
            if let Some(PropertyValue::Array(origins)) = node.properties.get("cors_allowed_origins")
            {
                return origins
                    .iter()
                    .filter_map(|v| {
                        if let PropertyValue::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
            }
        }
    }

    Vec::new()
}

/// Aggregate CORS allowed origins across all repos for a tenant.
///
/// Used for routes that don't carry a repo in the URL (e.g. `/api/uploads`).
#[cfg(feature = "storage-rocksdb")]
async fn get_all_cors_allowed_origins_for_tenant(
    storage: &std::sync::Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
) -> Vec<String> {
    use raisin_storage::{RepositoryManagementRepository, Storage};

    let repos = match storage
        .repository_management()
        .list_repositories_for_tenant(tenant_id)
        .await
    {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut all_origins = Vec::new();
    for repo in &repos {
        let origins = get_cors_allowed_origins_for_repo(storage, tenant_id, &repo.repo_id).await;
        for origin in origins {
            if !all_origins.contains(&origin) {
                all_origins.push(origin);
            }
        }
    }
    all_origins
}

/// Resolve CORS allowed origins using hierarchy: Repo > AllRepos > Tenant > Global.
///
/// Results are cached per `{tenant}/{repo}` (or `{tenant}/__all__` for repo-less routes).
#[cfg(feature = "storage-rocksdb")]
async fn resolve_cors_allowed_origins(
    storage: &std::sync::Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
    repo_id: Option<&str>,
    global_origins: &[String],
    cache: &raisin_core::TtlCache<Vec<String>>,
) -> Vec<String> {
    use raisin_rocksdb::repositories::TenantAuthConfigRepository;

    let cache_key = match repo_id {
        Some(repo) => format!("{}/{}", tenant_id, repo),
        None => format!("{}/__all__", tenant_id),
    };

    if let Some(cached) = cache.get(&cache_key) {
        return cached;
    }

    // 1. Check repo-level (highest priority)
    if let Some(repo) = repo_id {
        let repo_origins = get_cors_allowed_origins_for_repo(storage, tenant_id, repo).await;
        if !repo_origins.is_empty() {
            tracing::info!(
                tenant_id = %tenant_id,
                repo_id = %repo,
                origins = ?repo_origins,
                "CORS: Using repo-level config"
            );
            cache.put(&cache_key, repo_origins.clone());
            return repo_origins;
        }
    }

    // 1b. Aggregate all repos for tenant (no repo in URL)
    if repo_id.is_none() {
        let aggregated = get_all_cors_allowed_origins_for_tenant(storage, tenant_id).await;
        if !aggregated.is_empty() {
            tracing::info!(
                tenant_id = %tenant_id,
                origins = ?aggregated,
                "CORS: Using aggregated repo-level config (no repo in URL)"
            );
            cache.put(&cache_key, aggregated.clone());
            return aggregated;
        }
    }

    // 2. Check tenant-level
    if let Ok(Some(tenant_config)) = storage
        .tenant_auth_config_repository()
        .get_config(tenant_id)
        .await
    {
        if !tenant_config.cors_allowed_origins.is_empty() {
            tracing::info!(
                tenant_id = %tenant_id,
                origins = ?tenant_config.cors_allowed_origins,
                "CORS: Using tenant-level config"
            );
            cache.put(&cache_key, tenant_config.cors_allowed_origins.clone());
            return tenant_config.cors_allowed_origins;
        }
    }

    // 3. Fall back to global config
    tracing::info!(
        tenant_id = %tenant_id,
        origins = ?global_origins,
        "CORS: Using global config"
    );
    let result = global_origins.to_vec();
    cache.put(&cache_key, result.clone());
    result
}
