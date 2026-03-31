// SPDX-License-Identifier: BSL-1.1

//! Write (mutation) handlers for repository nodes.
//!
//! Handles PUT (update) and DELETE operations on nodes,
//! supporting both direct mode and commit/transaction mode.

use axum::{
    extract::{Extension, Json, Path, Query, State},
    http::StatusCode,
};
use raisin_models as models;
use raisin_models::auth::AuthContext;
use raisin_storage::{BranchRepository, NodeRepository, Storage, StorageScope};

use crate::{error::ApiError, middleware::RaisinContext, state::AppState, types::RepoQuery};

/// PUT handler for updating nodes.
///
/// Supports two modes:
/// - **Direct mode**: Updates HEAD without creating a revision
/// - **Commit mode**: Creates a transaction and commits with a revision
pub async fn repo_put(
    State(state): State<AppState>,
    Extension(ctx): Extension<RaisinContext>,
    Path((repo, branch, ws, _node_path)): Path<(String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Query(_q): Query<RepoQuery>,
    Json(json_body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);

    // Use cleaned_path from context (already has leading slash)
    let path = &ctx.cleaned_path;

    // If property_path exists, update property instead of full node
    if let Some(prop_path) = ctx.property_path {
        let value: models::nodes::properties::PropertyValue = serde_json::from_value(json_body)?;

        nodes_svc
            .update_property_by_path(&ctx.cleaned_path, &prop_path, value)
            .await?;

        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({"status": "property updated"})),
        ));
    }

    // Check for commit metadata (GitHub-style pattern)
    let commit_info: Option<crate::types::CommitInfo> = json_body
        .get("commit")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    // THIN HANDLER: Extract fields and delegate to service layer
    let mut properties: serde_json::Value = json_body
        .get("properties")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let translations = json_body
        .get("translations")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    if let Some(commit) = commit_info {
        // Commit mode: Use transaction to create revision
        // First get the node to find its ID
        let node = nodes_svc
            .get_by_path(path)
            .await?
            .ok_or_else(|| ApiError::node_not_found(path))?;

        // Prepare update properties with translations if provided
        if let Some(trans) = translations {
            // Add translations to properties update
            if let Some(obj) = properties.as_object_mut() {
                obj.insert(
                    "translations".to_string(),
                    serde_json::to_value(trans).unwrap_or_default(),
                );
            }
        }

        // Create transaction with single update operation
        // Transaction will fetch the node itself and apply updates
        let mut tx = nodes_svc.transaction();
        tx.update(node.id.clone(), properties.clone());

        let revision = tx.commit(commit.message, commit.actor).await?;

        // Fetch the updated node to return
        let updated_node = nodes_svc.get_by_path(path).await?;

        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "node": updated_node,
                "revision": revision,
                "committed": true
            })),
        ));
    }

    // Direct mode: Update HEAD without creating revision
    let mut builder = nodes_svc
        .update(&ws, path)
        .with_properties(serde_json::from_value(properties).unwrap_or_default());

    if let Some(trans) = translations {
        builder = builder.with_translations(trans);
    }

    let node = builder.save().await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::to_value(node).unwrap_or_default()),
    ))
}

/// DELETE handler for removing nodes.
///
/// Supports two modes:
/// - **Direct mode**: Deletes without creating a revision
/// - **Commit mode**: Creates a transaction with cascade delete and commits
pub async fn repo_delete(
    Extension(ctx): Extension<RaisinContext>,
    State(state): State<AppState>,
    Path((repo, branch, ws, _node_path)): Path<(String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    body: Option<Json<serde_json::Value>>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth

    // Get branch HEAD revision and bound queries to it for snapshot isolation
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let mut nodes_svc =
        state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);
    let branch_head = state
        .storage()
        .branches()
        .get_branch(tenant_id, &repo, &branch)
        .await?
        .map(|info| info.head);
    if let Some(head) = branch_head {
        nodes_svc = nodes_svc.at_revision(head);
    }

    // Use cleaned_path from context (already has leading slash)
    let path = &ctx.cleaned_path;

    // Check for commit metadata in request body (GitHub pattern)
    let commit_info: Option<crate::types::CommitInfo> = body
        .as_ref()
        .and_then(|json| json.get("commit"))
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    if let Some(commit) = commit_info {
        // Commit mode: Delete via transaction to create revision
        let node = nodes_svc
            .get_by_path(path)
            .await?
            .ok_or_else(|| ApiError::node_not_found(path))?;

        // Collect node + descendants for cascade delete commit
        let mut ids_to_delete = vec![node.id.clone()];
        let descendants = state
            .storage()
            .nodes()
            .deep_children_flat(
                StorageScope::new(tenant_id, &repo, &branch, &ws),
                path,
                100,
                branch_head.as_ref(),
            )
            .await?;
        for desc_node in descendants {
            ids_to_delete.push(desc_node.id);
        }

        let mut tx = nodes_svc.transaction();
        for id in &ids_to_delete {
            tx.delete(id.clone());
        }

        let revision = tx.commit(commit.message, commit.actor).await?;

        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "deleted": true,
                "node_id": node.id,
                "revision": revision,
                "committed": true
            })),
        ));
    }

    // Direct mode: Delete without creating revision
    let ok = nodes_svc.delete_by_path(path).await?;
    Ok((StatusCode::OK, Json(serde_json::json!({"deleted": ok}))))
}
