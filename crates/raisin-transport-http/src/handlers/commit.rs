use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;

use crate::{
    error::ApiError,
    state::AppState,
    types::{CommitNodeRequest, CommitResponse},
};
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Commit a single node update with a message (GitHub-like pattern)
///
/// POST /api/repository/{repo}/{branch}/{workspace}/content/{path}/raisin:cmd/save
///
/// Creates a new revision with the updated node properties.
pub async fn commit_node_save(
    State(state): State<AppState>,
    Path((repo, branch, ws, path)): Path<(String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<CommitNodeRequest>,
) -> Result<Json<CommitResponse>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth

    // Get the node to update
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);
    let node = nodes_svc
        .get_by_path(&path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(&path))?;

    // Create transaction
    let connection = state.connection();
    let tenant = connection.tenant(tenant_id);
    let repository = tenant.repository(&repo);
    let workspace = repository.workspace(&ws);
    let mut tx = workspace.nodes().branch(&branch).transaction();

    // Add update operation
    if let Some(properties) = req.properties {
        tx.update(node.id.clone(), properties);
    } else {
        return Err(ApiError::missing_required_field("properties"));
    }

    // Commit
    let revision = tx.commit(&req.message, &req.actor).await?;

    Ok(Json(CommitResponse {
        revision: revision.timestamp_ms,
        operations_count: Some(1),
    }))
}

/// Commit a single node creation with a message (GitHub-like pattern)
///
/// POST /api/repository/{repo}/{branch}/{workspace}/content/{path}/raisin:cmd/commit-create
///
/// Creates a new revision with the new node.
pub async fn commit_node_create(
    State(state): State<AppState>,
    Path((repo, branch, ws, _path)): Path<(String, String, String, String)>,
    Json(req): Json<CommitNodeRequest>,
) -> Result<Json<CommitResponse>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth

    // Parse node from request
    let node: raisin_models::nodes::Node = req
        .node
        .ok_or_else(|| ApiError::missing_required_field("node"))
        .and_then(|v| {
            serde_json::from_value(v).map_err(|e| ApiError::invalid_json(e.to_string()))
        })?;

    // Create transaction
    let connection = state.connection();
    let tenant = connection.tenant(tenant_id);
    let repository = tenant.repository(&repo);
    let workspace = repository.workspace(&ws);
    let mut tx = workspace.nodes().branch(&branch).transaction();

    // Add create operation
    tx.create(node);

    // Commit
    let revision = tx.commit(&req.message, &req.actor).await?;

    Ok(Json(CommitResponse {
        revision: revision.timestamp_ms,
        operations_count: Some(1),
    }))
}

/// Commit a single node deletion with a message (GitHub-like pattern)
///
/// DELETE /api/repository/{repo}/{branch}/{workspace}/content/{path}/raisin:cmd/delete
///
/// Creates a new revision with the node deleted.
pub async fn commit_node_delete(
    State(state): State<AppState>,
    Path((repo, branch, ws, path)): Path<(String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<CommitNodeRequest>,
) -> Result<Json<CommitResponse>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth

    // Get the node to delete
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);
    let node = nodes_svc
        .get_by_path(&path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(&path))?;

    // Create transaction
    let connection = state.connection();
    let tenant = connection.tenant(tenant_id);
    let repository = tenant.repository(&repo);
    let workspace = repository.workspace(&ws);
    let mut tx = workspace.nodes().branch(&branch).transaction();

    // Add delete operation
    // Cascade delete: collect node and all descendants, then add delete ops for each
    let mut ids_to_delete = vec![node.id.clone()];
    let descendants = state
        .storage()
        .nodes()
        .deep_children_flat(
            StorageScope::new(tenant_id, &repo, &branch, &ws),
            &path,
            100,
            None,
        )
        .await?;
    for desc_node in descendants {
        ids_to_delete.push(desc_node.id);
    }

    for id in ids_to_delete {
        tx.delete(id);
    }

    // Commit
    let revision = tx.commit(&req.message, &req.actor).await?;

    Ok(Json(CommitResponse {
        revision: revision.timestamp_ms,
        operations_count: Some(1),
    }))
}
