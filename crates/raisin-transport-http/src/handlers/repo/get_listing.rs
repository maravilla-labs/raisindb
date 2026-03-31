// SPDX-License-Identifier: BSL-1.1

//! Directory listing and GET command handlers.
//!
//! Handles directory listings with deep queries, pagination,
//! and GET-based commands (relations, list-translations, download).

use axum::{extract::Json, response::IntoResponse, response::Response};
use raisin_core::NodeService;
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_models::auth::AuthContext;
use raisin_storage::{transactional::TransactionalStorage, BranchRepository, Storage};

use crate::{error::ApiError, middleware::RaisinContext, state::AppState, types::RepoQuery};

use super::helpers::handle_file_download;
use super::translation_helpers::{
    resolve_array_with_locale, resolve_flat_with_locale, resolve_nested_with_locale,
    resolve_nodes_with_locale,
};

/// Handle GET commands (raisin:cmd pattern).
pub(super) async fn handle_get_command<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    ctx: &RaisinContext,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    path: &str,
    nodes_svc: &NodeService<S>,
    auth_context: Option<AuthContext>,
) -> Result<Response, ApiError> {
    let cmd_name = ctx.command_name.as_deref().unwrap_or("");
    match cmd_name {
        "relations" => {
            let relationships = nodes_svc.get_node_relationships(path).await?;
            Ok(Json(relationships).into_response())
        }
        "list-translations" => {
            handle_list_translations(state, nodes_svc, tenant_id, repo, branch, ws, path).await
        }
        "download" => {
            handle_file_download(
                state,
                tenant_id,
                repo,
                branch,
                ws,
                path,
                ctx.property_path.as_deref(),
                auth_context,
            )
            .await
        }
        _ => Err(ApiError::validation_failed(format!(
            "Unknown GET command: {}",
            cmd_name
        ))),
    }
}

/// Handle the `list-translations` GET command.
async fn handle_list_translations<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    path: &str,
) -> Result<Response, ApiError> {
    use raisin_storage::{Storage, TranslationRepository};

    let translation_repo = state.storage().translations();

    // Get current branch head revision
    let current_revision = state
        .storage()
        .branches()
        .get_head(tenant_id, repo, branch)
        .await?;

    // Get node to verify it exists
    let node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;

    // List translations for this node
    let locales = translation_repo
        .list_translations_for_node(tenant_id, repo, branch, ws, &node.id, &current_revision)
        .await?;

    let response = serde_json::json!({
        "node_id": node.id,
        "node_path": path,
        "locales": locales.iter().map(|l| l.as_str()).collect::<Vec<_>>()
    });

    Ok(Json(response).into_response())
}

/// Handle directory listing with optional deep queries and pagination.
pub(super) async fn handle_directory_listing<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    path: &str,
    q: &RepoQuery,
    revision: &HLC,
    nodes_svc: &NodeService<S>,
) -> Result<Response, ApiError> {
    let trimmed = path.trim_end_matches('/');

    // Check if pagination is requested
    if q.cursor.is_some() || q.limit.is_some() {
        let cursor = if let Some(cursor_str) = &q.cursor {
            Some(
                models::tree::PageCursor::decode(cursor_str)
                    .map_err(|e| ApiError::validation_failed(format!("Invalid cursor: {}", e)))?,
            )
        } else {
            None
        };

        let limit = q.limit.unwrap_or(100);
        let mut page = nodes_svc
            .list_children_page(trimmed, cursor.as_ref(), limit)
            .await?;

        // Apply translations if lang parameter is present
        page.items = resolve_nodes_with_locale(
            state,
            tenant_id,
            repo,
            branch,
            ws,
            page.items,
            q.lang.clone(),
            revision,
        )
        .await?;

        let response = serde_json::json!({
            "items": page.items,
            "next_cursor": page.next_cursor.as_ref().and_then(|c| c.encode().ok()),
            "total": page.total,
        });

        return Ok(Json(response).into_response());
    }

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
            let array = nodes_svc.deep_children_array(trimmed, depth).await?;
            let translated_array = resolve_array_with_locale(
                state,
                tenant_id,
                repo,
                branch,
                ws,
                array,
                q.lang.clone(),
                revision,
            )
            .await?;
            Ok(Json(translated_array).into_response())
        } else if q.flatten.unwrap_or(false) {
            let flat = nodes_svc.deep_children_flat(trimmed, depth).await?;
            let translated_flat = resolve_flat_with_locale(
                state,
                tenant_id,
                repo,
                branch,
                ws,
                flat,
                q.lang.clone(),
                revision,
            )
            .await?;
            Ok(Json(translated_flat).into_response())
        } else {
            let nested = nodes_svc.deep_children_nested(trimmed, depth).await?;
            let translated_nested = resolve_nested_with_locale(
                state,
                tenant_id,
                repo,
                branch,
                ws,
                nested,
                q.lang.clone(),
                revision,
            )
            .await?;
            Ok(Json(translated_nested).into_response())
        }
    } else {
        let children = nodes_svc.list_children(trimmed).await?;
        let translated_children = resolve_nodes_with_locale(
            state,
            tenant_id,
            repo,
            branch,
            ws,
            children,
            q.lang.clone(),
            revision,
        )
        .await?;
        Ok(Json(translated_children).into_response())
    }
}
