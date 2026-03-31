//! HTTP handlers for NodeType management
//!
//! Provides REST API endpoints for:
//! - CRUD operations on NodeTypes
//! - Publishing/unpublishing NodeTypes
//! - Listing and filtering NodeTypes
//! - Validation

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use raisin_core::{NodeTypeResolver, NodeValidator};
use raisin_models::nodes::types::NodeType;
use raisin_storage::scope::BranchScope;
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};

use crate::{error::ApiError, state::AppState};

#[derive(Debug, Deserialize)]
pub struct ValidateNodeRequest {
    pub workspace: String,
    pub node: raisin_models::nodes::Node,
}

#[derive(Debug, Serialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct NodeTypeCommitPayload {
    pub message: String,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub is_system: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct NodeTypeWriteRequest {
    pub node_type: NodeType,
    #[serde(default)]
    pub commit: Option<NodeTypeCommitPayload>,
}

fn resolve_commit(payload: Option<NodeTypeCommitPayload>, fallback: String) -> CommitMetadata {
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

/// Create a new NodeType
///
/// POST /api/management/:repo/:branch/nodetypes
#[axum::debug_handler]
pub async fn create_node_type(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Json(payload): Json<NodeTypeWriteRequest>,
) -> Result<(StatusCode, Json<NodeType>), ApiError> {
    let tenant_id = "default";
    let repo_id = &repo;
    let branch_name = &branch;

    let node_type = payload.node_type;

    // Validate the NodeType
    use validator::Validate;
    if node_type.validate().is_err() {
        return Err(ApiError::validation_failed("Invalid node type definition"));
    }

    // Validate initial_structure if present
    if node_type.initial_structure.is_some() {
        let tenant_id_owned = tenant_id.to_string();
        let repo_id_owned = repo_id.to_string();
        let branch_owned = branch_name.to_string();

        if node_type
            .validate_full(|name| {
                let storage = state.storage().clone();
                let tenant_id = tenant_id_owned.clone();
                let repo_id = repo_id_owned.clone();
                let branch = branch_owned.clone();

                async move {
                    let scope = BranchScope::new(&tenant_id, &repo_id, &branch);
                    storage
                        .node_types()
                        .get(scope, &name, None)
                        .await
                        .map(|opt| opt.is_some())
                        .map_err(|e| e.to_string())
                }
            })
            .await
            .is_err()
        {
            return Err(ApiError::validation_failed("Invalid initial structure"));
        }
    }

    let commit = resolve_commit(
        payload.commit,
        format!("Create node type {}", node_type.name),
    );

    let scope = BranchScope::new(tenant_id, repo_id, branch_name);

    let revision = state
        .storage()
        .node_types()
        .put(scope, node_type.clone(), commit)
        .await?;

    // Try to read back the created NodeType, but if it fails, return the original
    // This handles eventual consistency issues in distributed/cluster setups
    let stored = state
        .storage()
        .node_types()
        .get(scope, &node_type.name, Some(&revision))
        .await?
        .unwrap_or(node_type);

    Ok((StatusCode::CREATED, Json(stored)))
}

/// List all NodeTypes
///
/// GET /api/management/:repo/:branch/nodetypes
pub async fn list_node_types(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<Vec<NodeType>>, ApiError> {
    let tenant_id = "default";
    let repo_id = &repo;
    let branch_name = &branch;

    tracing::info!(
        target: "raisin_http::node_types",
        "list_node_types request: tenant={} repo={} branch={}",
        tenant_id,
        repo_id,
        branch_name
    );

    let scope = BranchScope::new(tenant_id, repo_id, branch_name);

    let node_types = state.storage().node_types().list(scope, None).await?;

    tracing::debug!(
        target: "raisin_http::node_types",
        "list_node_types returning {} entries for repo={} branch={}",
        node_types.len(),
        repo_id,
        branch_name
    );

    Ok(Json(node_types))
}

/// List only published NodeTypes
///
/// GET /api/management/:repo/:branch/nodetypes/published
pub async fn list_published_node_types(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<Vec<NodeType>>, ApiError> {
    let tenant_id = "default";
    let repo_id = &repo;
    let branch_name = &branch;

    tracing::info!(
        target: "raisin_http::node_types",
        "list_published_node_types request: tenant={} repo={} branch={}",
        tenant_id,
        repo_id,
        branch_name
    );

    let scope = BranchScope::new(tenant_id, repo_id, branch_name);

    let node_types = state
        .storage()
        .node_types()
        .list_published(scope, None)
        .await?;

    tracing::debug!(
        target: "raisin_http::node_types",
        "list_published_node_types returning {} entries for repo={} branch={}",
        node_types.len(),
        repo_id,
        branch_name
    );

    Ok(Json(node_types))
}

/// Get a specific NodeType by name
///
/// GET /api/management/:repo/:branch/nodetypes/:name
pub async fn get_node_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<Json<NodeType>, ApiError> {
    let tenant_id = "default";
    let repo_id = &repo;
    let branch_name = &branch;

    tracing::info!(
        target: "raisin_http::node_types",
        "get_node_type request: tenant={} repo={} branch={} name={}",
        tenant_id,
        repo_id,
        branch_name,
        name
    );

    let scope = BranchScope::new(tenant_id, repo_id, branch_name);

    let node_type = state
        .storage()
        .node_types()
        .get(scope, &name, None)
        .await?
        .ok_or_else(|| {
            tracing::warn!(
                target: "raisin_http::node_types",
                "get_node_type NOT FOUND: tenant={} repo={} branch={} name={}",
                tenant_id,
                repo_id,
                branch_name,
                name
            );
            ApiError::node_type_not_found(&name)
        })?;

    tracing::debug!(
        target: "raisin_http::node_types",
        "get_node_type returning schema id={:?} version={:?}",
        node_type.id,
        node_type.version
    );

    Ok(Json(node_type))
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct ResolvedNodeTypeQuery {
    pub workspace: Option<String>,
}

/// Get resolved NodeType with full inheritance applied
///
/// GET /api/management/:repo/:branch/nodetypes/:name/resolved
pub async fn get_resolved_node_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    Query(params): Query<ResolvedNodeTypeQuery>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant_id = "default";
    let resolver =
        NodeTypeResolver::new(state.storage().clone(), tenant_id.to_string(), repo, branch);
    let resolved = if let Some(workspace) = params.workspace.as_deref() {
        resolver.resolve_for_workspace(workspace, &name).await?
    } else {
        resolver.resolve(&name).await?
    };

    // Return as JSON with extra metadata
    let response = serde_json::json!({
        "node_type": resolved.node_type,
        "resolved_properties": resolved.resolved_properties,
        "resolved_allowed_children": resolved.resolved_allowed_children,
        "inheritance_chain": resolved.inheritance_chain,
    });

    Ok(Json(response))
}

/// Update a NodeType
///
/// PUT /api/management/:repo/:branch/nodetypes/:name
#[axum::debug_handler]
pub async fn update_node_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    Json(payload): Json<NodeTypeWriteRequest>,
) -> Result<Json<NodeType>, ApiError> {
    let tenant_id = "default";
    let repo_id = &repo;
    let branch_name = &branch;

    let mut node_type = payload.node_type;

    let scope = BranchScope::new(tenant_id, repo_id, branch_name);

    // Ensure target exists before updating
    let existing = state
        .storage()
        .node_types()
        .get(scope, &name, None)
        .await?
        .ok_or_else(|| ApiError::node_type_not_found(&name))?;

    // Preserve identifiers and creation metadata
    node_type.id = existing.id;
    node_type.name = name.clone();
    node_type.created_at = existing.created_at;

    use validator::Validate;
    node_type
        .validate()
        .map_err(|_| ApiError::validation_failed("Invalid node type definition"))?;

    if node_type.initial_structure.is_some() {
        let tenant_id_owned = tenant_id.to_string();
        let repo_id_owned = repo_id.to_string();
        let branch_owned = branch_name.to_string();

        node_type
            .validate_full(|name: String| {
                let storage = state.storage().clone();
                let tenant_id = tenant_id_owned.clone();
                let repo_id = repo_id_owned.clone();
                let branch = branch_owned.clone();

                async move {
                    let scope = BranchScope::new(&tenant_id, &repo_id, &branch);
                    storage
                        .node_types()
                        .get(scope, &name, None)
                        .await
                        .map(|opt| opt.is_some())
                        .map_err(|e| e.to_string())
                }
            })
            .await
            .map_err(|_| ApiError::validation_failed("Invalid initial structure"))?;
    }

    let commit = resolve_commit(
        payload.commit,
        format!("Update node type {}", node_type.name),
    );

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

/// Delete a NodeType
///
/// DELETE /api/management/:repo/:branch/nodetypes/:name
pub async fn delete_node_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    maybe_commit: Option<Json<NodeTypeCommitPayload>>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default";
    let repo_id = &repo;
    let branch_name = &branch;

    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Delete node type {}", name),
    );

    let scope = BranchScope::new(tenant_id, repo_id, branch_name);

    let deleted = state
        .storage()
        .node_types()
        .delete(scope, &name, commit)
        .await?;

    if deleted.is_some() {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::node_type_not_found(&name))
    }
}

/// Publish a NodeType
///
/// POST /api/management/:repo/:branch/nodetypes/:name/publish
pub async fn publish_node_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    maybe_commit: Option<Json<NodeTypeCommitPayload>>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default";
    let repo_id = &repo;
    let branch_name = &branch;

    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Publish node type {}", name),
    );

    let scope = BranchScope::new(tenant_id, repo_id, branch_name);

    state
        .storage()
        .node_types()
        .publish(scope, &name, commit)
        .await?;

    Ok(StatusCode::OK)
}

/// Unpublish a NodeType
///
/// POST /api/management/:repo/:branch/nodetypes/:name/unpublish
pub async fn unpublish_node_type(
    Path((repo, branch, name)): Path<(String, String, String)>,
    State(state): State<AppState>,
    maybe_commit: Option<Json<NodeTypeCommitPayload>>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default";
    let repo_id = &repo;
    let branch_name = &branch;

    let commit = resolve_commit(
        maybe_commit.map(|wrapper| wrapper.0),
        format!("Unpublish node type {}", name),
    );

    let scope = BranchScope::new(tenant_id, repo_id, branch_name);

    state
        .storage()
        .node_types()
        .unpublish(scope, &name, commit)
        .await?;

    Ok(StatusCode::OK)
}

/// Validate a node against its NodeType
///
/// POST /api/management/:repo/:branch/nodetypes/validate
pub async fn validate_node(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Json(req): Json<ValidateNodeRequest>,
) -> Result<Json<ValidationResult>, ApiError> {
    let tenant_id = "default";
    let repo_id = &repo;
    let branch_name = &branch;

    let validator = NodeValidator::new(
        state.storage().clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        branch_name.to_string(),
    );

    match validator.validate_node(&req.workspace, &req.node).await {
        Ok(()) => Ok(Json(ValidationResult {
            valid: true,
            errors: vec![],
        })),
        Err(e) => Ok(Json(ValidationResult {
            valid: false,
            errors: vec![e.to_string()],
        })),
    }
}
