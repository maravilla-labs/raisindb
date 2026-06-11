//! HTTP handlers for Mixin management
//!
//! Mixins are reusable property sets, stored as NodeTypes with `is_mixin = true`.
//! These endpoints mirror the NodeType handlers but scope every operation to
//! mixins: listing filters on `is_mixin`, and create/update force the flag on.
//!
//! Routes live under `/api/management/:repo/:branch/mixins`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};

use raisin_models::nodes::types::NodeType;
use raisin_storage::scope::BranchScope;
use raisin_storage::{NodeTypeRepository, Storage};

use super::node_types::{resolve_commit, NodeTypeCommitPayload, NodeTypeWriteRequest};
use crate::middleware::TenantInfo;
use crate::{error::ApiError, state::AppState};

/// True when a stored NodeType is a mixin.
fn is_mixin(node_type: &NodeType) -> bool {
    node_type.is_mixin == Some(true)
}

/// Create a new Mixin
///
/// POST /api/management/:repo/:branch/mixins
#[axum::debug_handler]
pub async fn create_mixin(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(payload): Json<NodeTypeWriteRequest>,
) -> Result<(StatusCode, Json<NodeType>), ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let scope = BranchScope::new(tenant_id, &repo, &branch);

    let mut node_type = payload.node_type;
    // A mixin is always flagged, regardless of what the client sent.
    node_type.is_mixin = Some(true);

    use validator::Validate;
    if node_type.validate().is_err() {
        return Err(ApiError::validation_failed("Invalid mixin definition"));
    }

    let commit = resolve_commit(payload.commit, format!("Create mixin {}", node_type.name));

    let revision = state
        .storage()
        .node_types()
        .put(scope, node_type.clone(), commit)
        .await?;

    let stored = state
        .storage()
        .node_types()
        .get(scope, &node_type.name, Some(&revision))
        .await?
        .unwrap_or(node_type);

    Ok((StatusCode::CREATED, Json(stored)))
}

/// List all Mixins
///
/// GET /api/management/:repo/:branch/mixins
pub async fn list_mixins(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<Vec<NodeType>>, ApiError> {
    let scope = BranchScope::new(tenant_info.tenant_id.as_str(), &repo, &branch);

    let mut node_types = state.storage().node_types().list(scope, None).await?;
    node_types.retain(is_mixin);

    Ok(Json(node_types))
}

/// List only published Mixins
///
/// GET /api/management/:repo/:branch/mixins/published
pub async fn list_published_mixins(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<Vec<NodeType>>, ApiError> {
    let scope = BranchScope::new(tenant_info.tenant_id.as_str(), &repo, &branch);

    let mut node_types = state
        .storage()
        .node_types()
        .list_published(scope, None)
        .await?;
    node_types.retain(is_mixin);

    Ok(Json(node_types))
}

/// Get a specific Mixin by name
///
/// GET /api/management/:repo/:branch/mixins/:name
pub async fn get_mixin(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<NodeType>, ApiError> {
    let scope = BranchScope::new(tenant_info.tenant_id.as_str(), &repo, &branch);

    let node_type = state
        .storage()
        .node_types()
        .get(scope, &name, None)
        .await?
        .filter(is_mixin)
        .ok_or_else(|| ApiError::node_type_not_found(&name))?;

    Ok(Json(node_type))
}

/// Update a Mixin
///
/// PUT /api/management/:repo/:branch/mixins/:name
#[axum::debug_handler]
pub async fn update_mixin(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(payload): Json<NodeTypeWriteRequest>,
) -> Result<Json<NodeType>, ApiError> {
    let scope = BranchScope::new(tenant_info.tenant_id.as_str(), &repo, &branch);

    // Ensure the target exists and is actually a mixin.
    let existing = state
        .storage()
        .node_types()
        .get(scope, &name, None)
        .await?
        .filter(is_mixin)
        .ok_or_else(|| ApiError::node_type_not_found(&name))?;

    let mut node_type = payload.node_type;
    node_type.id = existing.id;
    node_type.name = name.clone();
    node_type.created_at = existing.created_at;
    node_type.is_mixin = Some(true);

    use validator::Validate;
    node_type
        .validate()
        .map_err(|_| ApiError::validation_failed("Invalid mixin definition"))?;

    let commit = resolve_commit(payload.commit, format!("Update mixin {}", node_type.name));

    state
        .storage()
        .node_types()
        .put(scope, node_type.clone(), commit)
        .await?;

    let stored = state
        .storage()
        .node_types()
        .get(scope, &node_type.name, None)
        .await?
        .ok_or_else(|| ApiError::node_type_not_found(&node_type.name))?;

    Ok(Json(stored))
}

/// Delete a Mixin
///
/// DELETE /api/management/:repo/:branch/mixins/:name
pub async fn delete_mixin(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    maybe_commit: Option<Json<NodeTypeCommitPayload>>,
) -> Result<StatusCode, ApiError> {
    let scope = BranchScope::new(tenant_info.tenant_id.as_str(), &repo, &branch);

    // Only delete if it exists and is a mixin.
    state
        .storage()
        .node_types()
        .get(scope, &name, None)
        .await?
        .filter(is_mixin)
        .ok_or_else(|| ApiError::node_type_not_found(&name))?;

    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Delete mixin {}", name),
    );

    state
        .storage()
        .node_types()
        .delete(scope, &name, commit)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Publish a Mixin
///
/// POST /api/management/:repo/:branch/mixins/:name/publish
pub async fn publish_mixin(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    maybe_commit: Option<Json<NodeTypeCommitPayload>>,
) -> Result<StatusCode, ApiError> {
    let scope = BranchScope::new(tenant_info.tenant_id.as_str(), &repo, &branch);

    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Publish mixin {}", name),
    );

    state
        .storage()
        .node_types()
        .publish(scope, &name, commit)
        .await?;

    Ok(StatusCode::OK)
}

/// Unpublish a Mixin
///
/// POST /api/management/:repo/:branch/mixins/:name/unpublish
pub async fn unpublish_mixin(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    maybe_commit: Option<Json<NodeTypeCommitPayload>>,
) -> Result<StatusCode, ApiError> {
    let scope = BranchScope::new(tenant_info.tenant_id.as_str(), &repo, &branch);

    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Unpublish mixin {}", name),
    );

    state
        .storage()
        .node_types()
        .unpublish(scope, &name, commit)
        .await?;

    Ok(StatusCode::OK)
}
