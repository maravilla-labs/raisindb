// SPDX-License-Identifier: BSL-1.1

//! Revision-aware handlers for time-travel reads.
//!
//! These handlers allow reading nodes at a specific historical revision,
//! providing read-only access to past states of the content tree.

use axum::{
    extract::{Extension, Json, Path, Query, State},
    response::{IntoResponse, Response},
};
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_models::auth::AuthContext;

use crate::{error::ApiError, state::AppState, types::RepoQuery};

use super::translation_helpers::{
    resolve_array_with_locale, resolve_flat_with_locale, resolve_nested_with_locale,
    resolve_node_with_locale, resolve_nodes_with_locale,
};

/// Get root nodes at a specific revision (read-only)
///
/// GET /api/repository/{repo}/{branch}/rev/{revision}/{ws}/
pub async fn repo_get_root_at_revision(
    State(state): State<AppState>,
    Path((repo, branch, revision_str, ws)): Path<(String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<Vec<models::nodes::Node>>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth

    let revision: HLC = revision_str
        .parse()
        .map_err(|e| ApiError::validation_failed(format!("Invalid revision: {}", e)))?;

    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state
        .node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context)
        .at_revision(revision);

    let nodes = nodes_svc.list_root().await?;

    // Apply translations if lang parameter is present
    let nodes = resolve_nodes_with_locale(
        &state, tenant_id, &repo, &branch, &ws, nodes, q.lang, &revision,
    )
    .await?;

    Ok(Json(nodes))
}

/// Get node by ID at a specific revision (read-only)
///
/// GET /api/repository/{repo}/{branch}/rev/{revision}/{ws}/$ref/{id}
pub async fn repo_get_by_id_at_revision(
    State(state): State<AppState>,
    Path((repo, branch, revision_str, ws, id)): Path<(String, String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<models::nodes::Node>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth

    let revision: HLC = revision_str
        .parse()
        .map_err(|e| ApiError::validation_failed(format!("Invalid revision: {}", e)))?;

    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state
        .node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context)
        .at_revision(revision);

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
            // Node is hidden in this locale
            return Err(ApiError::node_not_found(&id));
        }
    };

    Ok(Json(node))
}

/// Get node by path at a specific revision (read-only)
///
/// GET /api/repository/{repo}/{branch}/rev/{revision}/{ws}/{*node_path}
///
/// Supports query parameters:
/// - level: int - depth for deep children queries (0-10)
/// - format: "array"|"nested"|"flat" - response format for deep queries
/// - flatten: bool - flatten deep results (alternative to format=flat)
pub async fn repo_get_at_revision(
    State(state): State<AppState>,
    Path((repo, branch, revision_str, ws, node_path)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<RepoQuery>,
) -> Result<Response, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth

    let revision: HLC = revision_str
        .parse()
        .map_err(|e| ApiError::validation_failed(format!("Invalid revision: {}", e)))?;

    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state
        .node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context)
        .at_revision(revision);

    // For revision routes, node_path becomes the path (with leading /)
    let path = if node_path.starts_with('/') {
        node_path.clone()
    } else {
        format!("/{}", node_path)
    };

    // Check if this is a directory listing request (ends with / or has level param)
    let is_dir = node_path.ends_with('/') || q.level.is_some();

    if is_dir {
        let trimmed = path.trim_end_matches('/');

        // Check if pagination is requested
        if q.cursor.is_some() || q.limit.is_some() {
            // Parse cursor if provided
            let cursor =
                if let Some(cursor_str) = &q.cursor {
                    Some(models::tree::PageCursor::decode(cursor_str).map_err(|e| {
                        ApiError::validation_failed(format!("Invalid cursor: {}", e))
                    })?)
                } else {
                    None
                };

            let limit = q.limit.unwrap_or(100);
            let mut page = nodes_svc
                .list_children_page(trimmed, cursor.as_ref(), limit)
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

        // Handle deep queries with level parameter
        if let Some(level) = q.level {
            let depth = level.min(10);

            // Verify parent exists if not root
            if trimmed != "/" && !trimmed.is_empty() {
                let exists = nodes_svc.get_by_path(trimmed).await?.is_some();
                if !exists {
                    return Err(ApiError::node_not_found(trimmed));
                }
            }

            // Check format parameter for DX-friendly array format
            if q.format.as_deref() == Some("array") || q.format.is_none() {
                // Default to array format for better DX
                let array = nodes_svc.deep_children_array(trimmed, depth).await?;
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
                let flat = nodes_svc.deep_children_flat(trimmed, depth).await?;
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
                let nested = nodes_svc.deep_children_nested(trimmed, depth).await?;
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

        // Simple children listing
        let children = nodes_svc.list_children(trimmed).await?;
        let translated_children = resolve_nodes_with_locale(
            &state,
            tenant_id,
            &repo,
            &branch,
            &ws,
            children,
            q.lang.clone(),
            &revision,
        )
        .await?;
        return Ok(Json(translated_children).into_response());
    }

    // Regular node GET at revision
    let node = nodes_svc
        .get_by_path(&path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(&path))?;

    // Apply translations if lang parameter is present
    let node = match resolve_node_with_locale(
        &state, tenant_id, &repo, &branch, &ws, node, q.lang, &revision,
    )
    .await?
    {
        Some(n) => n,
        None => {
            // Node is hidden in this locale
            return Err(ApiError::node_not_found(&path));
        }
    };

    Ok(Json(node).into_response())
}
