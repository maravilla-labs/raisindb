// SPDX-License-Identifier: BSL-1.1

//! GET request handlers for repository nodes.
//!
//! Handles reading nodes by root, by ID, and by path with support for
//! directory listings, deep queries, pagination, translations, and
//! version/property access.
//!
//! Directory listing and GET command logic is in [`super::get_listing`].

use axum::{
    body::Body,
    extract::{Extension, Json, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_storage::{BranchRepository, Storage};

use crate::{error::ApiError, middleware::RaisinContext, state::AppState, types::RepoQuery};

use super::assets::parse_asset_command_from_path;
use super::helpers::{get_node_version, get_property, handle_file_download, list_node_versions};
use super::translation_helpers::{
    resolve_array_with_locale, resolve_flat_with_locale, resolve_nested_with_locale,
    resolve_node_with_locale, resolve_nodes_with_locale,
};

/// GET handler for root nodes.
///
/// Returns children of the workspace root, supporting pagination,
/// deep queries, and translation resolution.
pub async fn repo_get_root(
    State(state): State<AppState>,
    Path((repo, branch, ws)): Path<(String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<RepoQuery>,
) -> Result<Response, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth
    tracing::info!(
        target: "raisin_http::repo",
        "repo_get_root request: tenant={} repo={} branch={} workspace={} cursor={:?} limit={:?} level={:?} format={:?} flatten={:?} lang={:?}",
        tenant_id,
        repo,
        branch,
        ws,
        q.cursor,
        q.limit,
        q.level,
        q.format,
        q.flatten,
        q.lang
    );

    // Get branch HEAD revision and bound queries to it for snapshot isolation
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let mut nodes_svc =
        state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);
    let revision = if let Some(branch_info) = state
        .storage()
        .branches()
        .get_branch(tenant_id, &repo, &branch)
        .await?
    {
        nodes_svc = nodes_svc.at_revision(branch_info.head);
        branch_info.head
    } else {
        HLC::new(0, 0)
    };

    // Check if pagination is requested
    if q.cursor.is_some() || q.limit.is_some() {
        let cursor = if let Some(cursor_str) = &q.cursor {
            Some(
                raisin_models::tree::PageCursor::decode(cursor_str)
                    .map_err(|e| ApiError::validation_failed(format!("Invalid cursor: {}", e)))?,
            )
        } else {
            None
        };

        let limit = q.limit.unwrap_or(100);
        let mut page = nodes_svc
            .list_children_page("/", cursor.as_ref(), limit)
            .await?;

        // Apply translations if lang parameter is present
        page.items = resolve_nodes_with_locale(
            &state,
            tenant_id,
            &repo,
            &branch,
            &ws,
            page.items,
            q.lang.clone(),
            &revision,
        )
        .await?;

        // Convert page to response with encoded cursor
        let response = serde_json::json!({
            "items": page.items,
            "next_cursor": page.next_cursor.as_ref().and_then(|c| c.encode().ok()),
            "total": page.total,
        });

        return Ok(Json(response).into_response());
    }

    // Handle deep queries
    if let Some(level) = q.level {
        let depth = level.min(10);

        if q.format.as_deref() == Some("array") || q.format.is_none() {
            let array = nodes_svc.deep_children_array("/", depth).await?;
            let translated_array = resolve_array_with_locale(
                &state,
                tenant_id,
                &repo,
                &branch,
                &ws,
                array,
                q.lang.clone(),
                &revision,
            )
            .await?;
            return Ok(Json(translated_array).into_response());
        } else if q.flatten.unwrap_or(false) || q.format.as_deref() == Some("flat") {
            let flat = nodes_svc.deep_children_flat("/", depth).await?;
            let translated_flat = resolve_flat_with_locale(
                &state,
                tenant_id,
                &repo,
                &branch,
                &ws,
                flat,
                q.lang.clone(),
                &revision,
            )
            .await?;
            return Ok(Json(translated_flat).into_response());
        } else {
            let nested = nodes_svc.deep_children_nested("/", depth).await?;
            let translated_nested = resolve_nested_with_locale(
                &state,
                tenant_id,
                &repo,
                &branch,
                &ws,
                nested,
                q.lang.clone(),
                &revision,
            )
            .await?;
            return Ok(Json(translated_nested).into_response());
        }
    }

    // Simple root listing
    let nodes = nodes_svc.list_root().await?;
    let nodes = resolve_nodes_with_locale(
        &state, tenant_id, &repo, &branch, &ws, nodes, q.lang, &revision,
    )
    .await?;

    Ok(Json(nodes).into_response())
}

/// GET handler for fetching a node by its ID.
pub async fn repo_get_by_id(
    State(state): State<AppState>,
    Path((repo, branch, ws, id)): Path<(String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<raisin_models::nodes::Node>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth

    // Get branch HEAD revision and bound queries to it for snapshot isolation
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let mut nodes_svc =
        state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);
    let revision = if let Some(branch_info) = state
        .storage()
        .branches()
        .get_branch(tenant_id, &repo, &branch)
        .await?
    {
        tracing::info!(
            target: "raisin_http::repo",
            "repo_get_by_id setting max_revision to branch HEAD: tenant={} repo={} branch={} head={}",
            tenant_id,
            repo,
            branch,
            branch_info.head
        );
        nodes_svc = nodes_svc.at_revision(branch_info.head);
        branch_info.head
    } else {
        HLC::new(0, 0)
    };

    let node = nodes_svc
        .get(&id)
        .await?
        .ok_or_else(|| ApiError::node_not_found(&id))?;

    // Apply translations if lang parameter is present
    let node = match resolve_node_with_locale(
        &state, tenant_id, &repo, &branch, &ws, node, q.lang, &revision,
    )
    .await?
    {
        Some(n) => n,
        None => {
            return Err(ApiError::node_not_found(&id));
        }
    };

    Ok(Json(node))
}

/// GET handler for fetching a node by path.
///
/// Supports:
/// - Single node retrieval
/// - Directory listings (trailing slash or level param)
/// - Deep queries with format options (array, nested, flat)
/// - Pagination via cursor/limit
/// - Version access via raisin:versions pattern
/// - Property access via @property notation
/// - Asset download via raisin:download/raisin:display commands
/// - YAML response format
pub async fn repo_get(
    Extension(ctx): Extension<RaisinContext>,
    State(state): State<AppState>,
    Path((repo, branch, ws, node_path)): Path<(String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<RepoQuery>,
) -> Result<Response, ApiError> {
    // Check for raisin:download or raisin:display commands in the path
    if let Some((asset_path, command)) = parse_asset_command_from_path(&ctx.cleaned_path) {
        let sig = q.sig.as_deref().unwrap_or_default();
        let exp = q.exp.unwrap_or(0);
        let property_path = ctx.property_path.as_deref();

        return super::assets::handle_asset_command_internal(
            &state,
            &repo,
            &branch,
            &ws,
            &asset_path,
            &command,
            property_path,
            sig,
            exp,
        )
        .await;
    }

    let tenant_id = "default"; // TODO: Extract from middleware/auth

    // Get branch HEAD revision and bound queries to it for snapshot isolation
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let mut nodes_svc =
        state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context.clone());
    let revision = if let Some(branch_info) = state
        .storage()
        .branches()
        .get_branch(tenant_id, &repo, &branch)
        .await?
    {
        tracing::info!(
            target: "raisin_http::repo",
            "repo_get setting max_revision to branch HEAD: tenant={} repo={} branch={} head={}",
            tenant_id,
            repo,
            branch,
            branch_info.head
        );
        nodes_svc = nodes_svc.at_revision(branch_info.head);
        branch_info.head
    } else {
        tracing::warn!(
            "HTTP Handler: Branch not found, no max_revision set: tenant={}, repo={}, branch={}",
            tenant_id,
            repo,
            branch
        );
        HLC::new(0, 0) // Default to revision 0 if branch not found
    };

    // Use cleaned_path from context (already has leading slash)
    let path = &ctx.cleaned_path;

    // Handle version requests
    if ctx.is_version {
        if let Some(vid) = ctx.version_id {
            return get_node_version(
                &state,
                tenant_id,
                &repo,
                &branch,
                &ws,
                path,
                vid,
                auth_context.clone(),
            )
            .await;
        } else {
            return list_node_versions(
                &state,
                tenant_id,
                &repo,
                &branch,
                &ws,
                path,
                auth_context.clone(),
            )
            .await;
        }
    }

    // Handle property path access
    if let Some(prop_path) = &ctx.property_path {
        return get_property(
            &state,
            tenant_id,
            &repo,
            &branch,
            &ws,
            path,
            prop_path,
            auth_context.clone(),
        )
        .await;
    }

    // Handle command requests (raisin:cmd pattern)
    if ctx.is_command {
        return super::get_listing::handle_get_command(
            &state,
            &ctx,
            tenant_id,
            &repo,
            &branch,
            &ws,
            path,
            &nodes_svc,
            auth_context.clone(),
        )
        .await;
    }

    // Handle file download request via query parameter (legacy/alternative syntax)
    if q.command.as_deref() == Some("download") {
        return handle_file_download(
            &state,
            tenant_id,
            &repo,
            &branch,
            &ws,
            path,
            ctx.property_path.as_deref(),
            auth_context.clone(),
        )
        .await;
    }

    // Determine lookup path based on extension.
    let lookup_path = match ctx.file_extension.as_deref() {
        Some("yaml") | Some("yml") => path.clone(),
        Some(ext) => {
            if path.ends_with(&format!(".{}", ext)) {
                path.clone()
            } else {
                format!("{}.{}", path, ext)
            }
        }
        None => path.clone(),
    };

    // Check if this is a directory listing request (ends with / or has level param)
    let is_dir = node_path.ends_with('/') || q.level.is_some();
    tracing::debug!(
        target: "raisin_http::repo",
        "repo_get lookup context: lookup_path='{}' cleaned_path='{}' file_extension={:?} is_dir={} level={:?}",
        lookup_path,
        path,
        ctx.file_extension,
        is_dir,
        q.level
    );

    if is_dir {
        return super::get_listing::handle_directory_listing(
            &state, tenant_id, &repo, &branch, &ws, path, &q, &revision, &nodes_svc,
        )
        .await;
    }

    // Get single node
    let node = match nodes_svc.get_by_path(&lookup_path).await? {
        Some(node) => {
            tracing::debug!(
                target: "raisin_http::repo",
                "repo_get found node id={} path='{}' repo={} branch={} workspace={}",
                node.id,
                node.path,
                repo,
                branch,
                ws
            );
            node
        }
        None => {
            tracing::warn!(
                target: "raisin_http::repo",
                "repo_get NOT FOUND: tenant={} repo={} branch={} workspace={} lookup_path='{}' branch_head={}",
                tenant_id,
                repo,
                branch,
                ws,
                lookup_path,
                revision
            );
            return Err(ApiError::node_not_found(&lookup_path));
        }
    };

    // Apply translations if lang parameter is present
    let node = match resolve_node_with_locale(
        &state,
        tenant_id,
        &repo,
        &branch,
        &ws,
        node,
        q.lang.clone(),
        &revision,
    )
    .await?
    {
        Some(n) => n,
        None => {
            // Node is hidden in this locale
            return Err(ApiError::node_not_found(&lookup_path));
        }
    };

    // Handle YAML response format
    if ctx.file_extension.as_deref() == Some("yaml") || ctx.file_extension.as_deref() == Some("yml")
    {
        let yaml = serde_yaml::to_string(&node)
            .map_err(|e| ApiError::serialization_error(e.to_string()))?;
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/yaml; charset=utf-8")
            .body(Body::from(yaml))
            .expect("valid response with valid headers"));
    }

    // Default JSON response
    Ok(Json(node).into_response())
}
