//! HTTP handlers for Archetype management

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use raisin_core::ArchetypeResolver;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_storage::scope::BranchScope;
use raisin_storage::{ArchetypeRepository, CommitMetadata, Storage};

use crate::{error::ApiError, state::AppState};

#[derive(Debug, Deserialize)]
pub struct ArchetypeCommitPayload {
    pub message: String,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub is_system: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ArchetypeWriteRequest {
    pub archetype: Archetype,
    #[serde(default)]
    pub commit: Option<ArchetypeCommitPayload>,
}

fn resolve_commit(payload: Option<ArchetypeCommitPayload>, fallback: String) -> CommitMetadata {
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

/// Create a new Archetype
///
/// POST /api/management/:repo/:branch/archetypes
pub async fn create_archetype(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Json(payload): Json<ArchetypeWriteRequest>,
) -> Result<(StatusCode, Json<Archetype>), ApiError> {
    let tenant_id = "default";
    let archetype = payload.archetype;

    use validator::Validate;
    if let Err(err) = archetype.validate() {
        return Err(ApiError::validation_failed("Invalid archetype definition")
            .with_details(err.to_string()));
    }

    let commit = resolve_commit(
        payload.commit,
        format!("Create archetype {}", archetype.name),
    );

    state
        .storage()
        .archetypes()
        .put(
            BranchScope::new(tenant_id, &repo, &branch),
            archetype.clone(),
            commit,
        )
        .await?;

    let stored = state
        .storage()
        .archetypes()
        .get(
            BranchScope::new(tenant_id, &repo, &branch),
            &archetype.name,
            None,
        )
        .await?
        .ok_or_else(|| ApiError::archetype_not_found(&archetype.name))?;

    Ok((StatusCode::CREATED, Json(stored)))
}

/// List all Archetypes
///
/// GET /api/management/:repo/:branch/archetypes
pub async fn list_archetypes(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<Vec<Archetype>>, ApiError> {
    let tenant_id = "default";

    let archetypes = state
        .storage()
        .archetypes()
        .list(BranchScope::new(tenant_id, &repo, &branch), None)
        .await?;

    Ok(Json(archetypes))
}

/// List published Archetypes
///
/// GET /api/management/:repo/:branch/archetypes/published
pub async fn list_published_archetypes(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<Vec<Archetype>>, ApiError> {
    let tenant_id = "default";

    let archetypes = state
        .storage()
        .archetypes()
        .list_published(BranchScope::new(tenant_id, &repo, &branch), None)
        .await?;

    Ok(Json(archetypes))
}

/// Get a single Archetype by name
///
/// GET /api/management/:repo/:branch/archetypes/:name
pub async fn get_archetype(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<Json<Archetype>, ApiError> {
    let tenant_id = "default";

    let archetype = state
        .storage()
        .archetypes()
        .get(BranchScope::new(tenant_id, &repo, &branch), &name, None)
        .await?
        .ok_or_else(|| ApiError::archetype_not_found(name.clone()))?;

    Ok(Json(archetype))
}

/// Update an existing Archetype
///
/// PUT /api/management/:repo/:branch/archetypes/:name
pub async fn update_archetype(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    Json(payload): Json<ArchetypeWriteRequest>,
) -> Result<Json<Archetype>, ApiError> {
    let tenant_id = "default";
    let archetype = payload.archetype;

    if archetype.name != name {
        return Err(ApiError::validation_failed(
            "Archetype name in payload does not match path parameter",
        ));
    }

    use validator::Validate;
    if let Err(err) = archetype.validate() {
        return Err(ApiError::validation_failed("Invalid archetype definition")
            .with_details(err.to_string()));
    }

    let commit = resolve_commit(
        payload.commit,
        format!("Update archetype {}", archetype.name),
    );

    state
        .storage()
        .archetypes()
        .put(
            BranchScope::new(tenant_id, &repo, &branch),
            archetype.clone(),
            commit,
        )
        .await?;

    let stored = state
        .storage()
        .archetypes()
        .get(BranchScope::new(tenant_id, &repo, &branch), &name, None)
        .await?
        .ok_or_else(|| ApiError::archetype_not_found(name.clone()))?;

    Ok(Json(stored))
}

/// Delete an Archetype by name
///
/// DELETE /api/management/:repo/:branch/archetypes/:name
pub async fn delete_archetype(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    maybe_commit: Option<Json<ArchetypeCommitPayload>>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default";
    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Delete archetype {}", name),
    );

    let deleted = state
        .storage()
        .archetypes()
        .delete(BranchScope::new(tenant_id, &repo, &branch), &name, commit)
        .await?;

    if deleted.is_none() {
        return Err(ApiError::archetype_not_found(name));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Publish an Archetype
///
/// POST /api/management/:repo/:branch/archetypes/:name/publish
pub async fn publish_archetype(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    maybe_commit: Option<Json<ArchetypeCommitPayload>>,
) -> Result<Json<Archetype>, ApiError> {
    let tenant_id = "default";
    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Publish archetype {}", name),
    );

    state
        .storage()
        .archetypes()
        .publish(BranchScope::new(tenant_id, &repo, &branch), &name, commit)
        .await?;

    let archetype = state
        .storage()
        .archetypes()
        .get(BranchScope::new(tenant_id, &repo, &branch), &name, None)
        .await?
        .ok_or_else(|| ApiError::archetype_not_found(name.clone()))?;

    Ok(Json(archetype))
}

/// Get resolved Archetype with all inheritance applied
///
/// GET /api/management/:repo/:branch/archetypes/:name/resolved
pub async fn get_resolved_archetype(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant_id = "default";
    let resolver =
        ArchetypeResolver::new(state.storage().clone(), tenant_id.to_string(), repo, branch);
    let resolved = resolver.resolve(&name).await?;

    let response = serde_json::json!({
        "archetype": resolved.archetype,
        "resolved_fields": resolved.resolved_fields,
        "resolved_layout": resolved.resolved_layout,
        "inheritance_chain": resolved.inheritance_chain,
        "resolved_strict": resolved.resolved_strict,
    });

    Ok(Json(response))
}

/// Unpublish an Archetype
///
/// POST /api/management/:repo/:branch/archetypes/:name/unpublish
pub async fn unpublish_archetype(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    maybe_commit: Option<Json<ArchetypeCommitPayload>>,
) -> Result<Json<Archetype>, ApiError> {
    let tenant_id = "default";
    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Unpublish archetype {}", name),
    );

    state
        .storage()
        .archetypes()
        .unpublish(BranchScope::new(tenant_id, &repo, &branch), &name, commit)
        .await?;

    let archetype = state
        .storage()
        .archetypes()
        .get(BranchScope::new(tenant_id, &repo, &branch), &name, None)
        .await?
        .ok_or_else(|| ApiError::archetype_not_found(name.clone()))?;

    Ok(Json(archetype))
}
