//! HTTP handlers for ElementType management

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use raisin_core::ElementTypeResolver;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_storage::scope::BranchScope;
use raisin_storage::{CommitMetadata, ElementTypeRepository, Storage};

use crate::{error::ApiError, state::AppState};

#[derive(Debug, Deserialize)]
pub struct ElementTypeCommitPayload {
    pub message: String,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub is_system: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ElementTypeWriteRequest {
    pub element_type: ElementType,
    #[serde(default)]
    pub commit: Option<ElementTypeCommitPayload>,
}

fn resolve_commit(payload: Option<ElementTypeCommitPayload>, fallback: String) -> CommitMetadata {
    match payload {
        Some(p) => CommitMetadata {
            message: p.message,
            actor: p.actor.unwrap_or_else(|| "system".to_string()),
            is_system: p.is_system.unwrap_or(false),
        },
        None => CommitMetadata {
            message: fallback,
            actor: "system".to_string(),
            is_system: true,
        },
    }
}

/// Create a new ElementType
pub async fn create_element_type(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Json(payload): Json<ElementTypeWriteRequest>,
) -> Result<(StatusCode, Json<ElementType>), ApiError> {
    let tenant_id = "default";
    let element_type = payload.element_type;

    let commit = resolve_commit(
        payload.commit,
        format!("Create element type {}", element_type.name),
    );

    state
        .storage()
        .element_types()
        .put(
            BranchScope::new(tenant_id, &repo, &branch),
            element_type.clone(),
            commit,
        )
        .await?;

    let stored = state
        .storage()
        .element_types()
        .get(
            BranchScope::new(tenant_id, &repo, &branch),
            &element_type.name,
            None,
        )
        .await?
        .ok_or_else(|| ApiError::element_type_not_found(&element_type.name))?;

    Ok((StatusCode::CREATED, Json(stored)))
}

/// List all ElementTypes
pub async fn list_element_types(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ElementType>>, ApiError> {
    let tenant_id = "default";

    let element_types = state
        .storage()
        .element_types()
        .list(BranchScope::new(tenant_id, &repo, &branch), None)
        .await?;

    Ok(Json(element_types))
}

/// List published ElementTypes
pub async fn list_published_element_types(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ElementType>>, ApiError> {
    let tenant_id = "default";

    let element_types = state
        .storage()
        .element_types()
        .list_published(BranchScope::new(tenant_id, &repo, &branch), None)
        .await?;

    Ok(Json(element_types))
}

/// Get a single ElementType
pub async fn get_element_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<Json<ElementType>, ApiError> {
    let tenant_id = "default";

    let element_type = state
        .storage()
        .element_types()
        .get(BranchScope::new(tenant_id, &repo, &branch), &name, None)
        .await?
        .ok_or_else(|| ApiError::element_type_not_found(name.clone()))?;

    Ok(Json(element_type))
}

/// Update an ElementType
pub async fn update_element_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    Json(payload): Json<ElementTypeWriteRequest>,
) -> Result<Json<ElementType>, ApiError> {
    let tenant_id = "default";
    let element_type = payload.element_type;

    if element_type.name != name {
        return Err(ApiError::validation_failed(
            "Element type name in payload does not match path parameter",
        ));
    }

    let commit = resolve_commit(
        payload.commit,
        format!("Update element type {}", element_type.name),
    );

    state
        .storage()
        .element_types()
        .put(
            BranchScope::new(tenant_id, &repo, &branch),
            element_type.clone(),
            commit,
        )
        .await?;

    let stored = state
        .storage()
        .element_types()
        .get(BranchScope::new(tenant_id, &repo, &branch), &name, None)
        .await?
        .ok_or_else(|| ApiError::element_type_not_found(name.clone()))?;

    Ok(Json(stored))
}

/// Delete an ElementType
pub async fn delete_element_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    maybe_commit: Option<Json<ElementTypeCommitPayload>>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default";
    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Delete element type {}", name),
    );

    let deleted = state
        .storage()
        .element_types()
        .delete(BranchScope::new(tenant_id, &repo, &branch), &name, commit)
        .await?;

    if deleted.is_none() {
        return Err(ApiError::element_type_not_found(name));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Publish an ElementType
pub async fn publish_element_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    maybe_commit: Option<Json<ElementTypeCommitPayload>>,
) -> Result<Json<ElementType>, ApiError> {
    let tenant_id = "default";
    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Publish element type {}", name),
    );

    state
        .storage()
        .element_types()
        .publish(BranchScope::new(tenant_id, &repo, &branch), &name, commit)
        .await?;

    let element_type = state
        .storage()
        .element_types()
        .get(BranchScope::new(tenant_id, &repo, &branch), &name, None)
        .await?
        .ok_or_else(|| ApiError::element_type_not_found(name.clone()))?;

    Ok(Json(element_type))
}

/// Get resolved ElementType with all inheritance applied
///
/// GET /api/management/:repo/:branch/elementtypes/:name/resolved
pub async fn get_resolved_element_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant_id = "default";
    let resolver =
        ElementTypeResolver::new(state.storage().clone(), tenant_id.to_string(), repo, branch);
    let resolved = resolver.resolve(&name).await?;

    let response = serde_json::json!({
        "element_type": resolved.element_type,
        "resolved_fields": resolved.resolved_fields,
        "resolved_layout": resolved.resolved_layout,
        "inheritance_chain": resolved.inheritance_chain,
        "resolved_strict": resolved.resolved_strict,
    });

    Ok(Json(response))
}

/// Unpublish an ElementType
pub async fn unpublish_element_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    maybe_commit: Option<Json<ElementTypeCommitPayload>>,
) -> Result<Json<ElementType>, ApiError> {
    let tenant_id = "default";
    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Unpublish element type {}", name),
    );

    state
        .storage()
        .element_types()
        .unpublish(BranchScope::new(tenant_id, &repo, &branch), &name, commit)
        .await?;

    let element_type = state
        .storage()
        .element_types()
        .get(BranchScope::new(tenant_id, &repo, &branch), &name, None)
        .await?
        .ok_or_else(|| ApiError::element_type_not_found(name.clone()))?;

    Ok(Json(element_type))
}
