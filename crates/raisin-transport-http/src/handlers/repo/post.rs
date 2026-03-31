// SPDX-License-Identifier: BSL-1.1

//! POST request handlers for repository nodes.
//!
//! Handles creating new nodes, file uploads (multipart and inline),
//! command execution, and sign URL generation.
//! Upload logic is split into [`super::post_multipart`] and [`super::post_external`].

use axum::{
    body::{Body, Bytes},
    extract::{Extension, Json, Path, Query, State},
    http::{header, StatusCode},
};
use http_body_util::BodyExt;
use raisin_core::NodeService;
use raisin_models as models;
use raisin_models::auth::AuthContext;
use raisin_storage::{transactional::TransactionalStorage, Storage};

use crate::{
    error::ApiError,
    middleware::RaisinContext,
    state::AppState,
    types::{CommandBody, RepoQuery},
};

use super::assets::{parse_sign_command_from_path, SignAssetRequest};

/// Threshold for switching from buffered to streaming upload (100MB)
const BUFFER_THRESHOLD: u64 = 100 * 1024 * 1024;

/// POST handler for creating nodes at the root level.
#[axum::debug_handler]
pub async fn repo_post_root(
    State(state): State<AppState>,
    Path((repo, branch, ws)): Path<(String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Json(json_body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth

    tracing::info!(
        "POST root: tenant={}, repo={}, branch={}, ws={}",
        tenant_id,
        repo,
        branch,
        ws
    );

    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);

    // Check for commit metadata
    let commit_info: Option<crate::types::CommitInfo> = json_body
        .get("commit")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    // Extract node data (try both root "node" field and direct properties)
    let mut node: models::nodes::Node = if let Some(node_val) = json_body.get("node") {
        serde_json::from_value(node_val.clone())?
    } else {
        // Assume entire body is the node (backward compatible)
        serde_json::from_value(json_body.clone())?
    };

    tracing::info!(
        "Node data: name={}, node_type={}",
        node.name,
        node.node_type
    );

    if let Some(commit) = commit_info {
        // Commit mode: Create node via transaction
        node.created_by = Some(commit.actor.clone());
        node.updated_by = Some(commit.actor.clone());

        // Ensure node has required fields
        if node.id.is_empty() {
            node.id = nanoid::nanoid!();
        }
        if node.path.is_empty() {
            node.path = format!("/{}", node.name);
        }

        let node_id = node.id.clone();

        let mut tx = nodes_svc.transaction();
        tx.create(node);

        let revision = tx.commit(commit.message, commit.actor).await?;

        // Fetch the created node at the specific revision it was created at
        let created_node = nodes_svc.at_revision(revision).get(&node_id).await?;

        return Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({
                "node": created_node,
                "revision": revision,
                "committed": true
            })),
        ));
    }

    // Direct mode: Add node without creating revision
    tracing::info!("Calling add_node with path='/'");
    let n = nodes_svc.add_node("/", node).await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::to_value(n).unwrap_or_default()),
    ))
}

/// POST handler for creating/uploading nodes at a specific path.
///
/// Supports:
/// - JSON node creation (with optional commit)
/// - Multipart file uploads (inline and external storage)
/// - Large file streaming (>100MB threshold)
/// - Command execution via raisin:cmd pattern
/// - Asset URL signing via raisin:sign pattern
pub async fn repo_post(
    State(state): State<AppState>,
    Extension(ctx): Extension<RaisinContext>,
    Path((repo, branch, ws, _node_path)): Path<(String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<RepoQuery>,
    request: axum::http::Request<Body>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    // Check for raisin:sign command in the path
    if let Some(asset_path) = parse_sign_command_from_path(&ctx.cleaned_path) {
        let body_bytes = request
            .into_body()
            .collect()
            .await
            .map_err(|e| ApiError::validation_failed(format!("Failed to read body: {}", e)))?
            .to_bytes();

        let sign_request: SignAssetRequest = serde_json::from_slice(&body_bytes)
            .map_err(|e| ApiError::validation_failed(format!("Invalid JSON: {}", e)))?;

        let response = super::assets::sign_asset_url_internal(
            &state,
            &ctx,
            &repo,
            &branch,
            &ws,
            &asset_path,
            sign_request,
        )
        .await?;
        return Ok((StatusCode::OK, Json(serde_json::to_value(response.0)?)));
    }

    let tenant_id = "default"; // TODO: Extract from middleware/auth
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc =
        state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context.clone());

    // Use cleaned_path from context (already has leading slash)
    let path = ctx.cleaned_path.clone();

    // Extract headers before consuming request
    let headers = request.headers().clone();
    let archetype_header = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Check content length for streaming decision
    let content_length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    let is_large_upload = content_length
        .map(|len| len > BUFFER_THRESHOLD)
        .unwrap_or(false);
    let is_multipart = archetype_header.contains("multipart/form-data");

    // For large multipart uploads, handle streaming directly without buffering
    if is_large_upload && is_multipart {
        return super::upload::handle_large_multipart_upload(
            state,
            ctx,
            repo,
            branch,
            ws,
            path.clone(),
            q,
            request,
            &archetype_header,
            auth_context,
            tenant_id,
        )
        .await;
    }

    // Collect body into bytes for small uploads and non-multipart requests
    let body = request.into_body();
    let bytes = body
        .collect()
        .await
        .map_err(|e| ApiError::validation_failed(format!("Failed to read request body: {}", e)))?
        .to_bytes();

    // Handle command execution from context (raisin:cmd marker)
    if ctx.is_command {
        let cmd_name = ctx.command_name.as_deref().unwrap_or("");
        let cmd_body: CommandBody = serde_json::from_slice(&bytes).map_err(|e| {
            tracing::error!("Failed to deserialize command body: {:?}", e);
            tracing::error!(
                "Command: {}, Body bytes: {}",
                cmd_name,
                String::from_utf8_lossy(&bytes)
            );
            ApiError::invalid_json(e.to_string())
        })?;
        return super::commands::repo_execute_command(
            &state,
            tenant_id,
            &repo,
            &branch,
            &ws,
            &ctx.cleaned_path,
            cmd_name,
            cmd_body,
            auth_context.clone(),
        )
        .await;
    }

    // Handle command execution from query parameter
    if let Some(cmd) = q.command.as_deref() {
        let cmd_body: CommandBody = serde_json::from_slice(&bytes).map_err(|e| {
            tracing::error!("Failed to deserialize command body (query param): {:?}", e);
            tracing::error!(
                "Command: {}, Body bytes: {}",
                cmd,
                String::from_utf8_lossy(&bytes)
            );
            ApiError::invalid_json(e.to_string())
        })?;
        return super::commands::repo_execute_command(
            &state,
            tenant_id,
            &repo,
            &branch,
            &ws,
            &path,
            cmd,
            cmd_body,
            auth_context,
        )
        .await;
    }

    if archetype_header.contains("multipart/form-data") {
        return super::post_multipart::handle_multipart_upload(
            &state,
            &nodes_svc,
            &ctx,
            &repo,
            &branch,
            &ws,
            &path,
            &q,
            &archetype_header,
            bytes,
            auth_context,
            tenant_id,
        )
        .await;
    }

    // Non-multipart: Parse JSON body (could be just node, or {node, commit})
    handle_json_post(
        &state,
        &nodes_svc,
        &path,
        &repo,
        &branch,
        &ws,
        bytes,
        auth_context,
        tenant_id,
    )
    .await
}

/// Handle JSON POST for node creation.
async fn handle_json_post<S: Storage + TransactionalStorage + 'static>(
    _state: &AppState,
    nodes_svc: &NodeService<S>,
    path: &str,
    _repo: &str,
    _branch: &str,
    _ws: &str,
    bytes: Bytes,
    auth_context: Option<AuthContext>,
    tenant_id: &str,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let json_body: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| ApiError::invalid_json(e.to_string()))?;

    // Check for commit metadata, provide defaults if missing
    let commit_info: Option<crate::types::CommitInfo> = json_body
        .get("commit")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    // Extract node data
    let mut node: models::nodes::Node = if let Some(node_val) = json_body.get("node") {
        serde_json::from_value(node_val.clone())?
    } else {
        serde_json::from_value(json_body.clone())?
    };

    let parent_path = path.trim_end_matches('/').to_string();

    tracing::info!(
        "POST child node: tenant={}, parent_path='{}', name='{}', type='{}'",
        tenant_id,
        parent_path,
        node.name,
        node.node_type
    );

    // ALWAYS use transaction mode (provide default commit info if not specified)
    let default_actor = auth_context
        .as_ref()
        .map(|ctx| ctx.actor_id())
        .unwrap_or_else(|| "system".to_string());
    let commit = commit_info.unwrap_or_else(|| crate::types::CommitInfo {
        message: format!("Create node: {}", node.name),
        actor: default_actor,
    });

    node.created_by = Some(commit.actor.clone());
    node.updated_by = Some(commit.actor.clone());

    if node.id.is_empty() {
        node.id = nanoid::nanoid!();
    }
    if node.path.is_empty() {
        let clean_name = raisin_core::sanitize_name(&node.name)?;
        node.path = if parent_path.is_empty() || parent_path == "/" {
            format!("/{}", clean_name)
        } else {
            format!("{}/{}", parent_path, clean_name)
        };
    }

    let node_id = node.id.clone();

    let mut tx = nodes_svc.transaction();
    tx.create(node);

    let revision = tx.commit(commit.message, commit.actor).await?;

    // Read back the created node (service already has the revision context)
    let created_node = nodes_svc.get(&node_id).await?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "node": created_node,
            "revision": revision,
            "committed": true
        })),
    ))
}
