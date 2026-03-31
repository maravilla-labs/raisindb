//! Registry management HTTP handlers
//!
//! These endpoints manage tenant and deployment registration in a multi-tenant environment.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use raisin_storage::{RegistryRepository, Storage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::state::AppState;

/// Response for tenant registration info
#[derive(Debug, Serialize, Deserialize)]
pub struct TenantResponse {
    pub tenant_id: String,
    pub created_at: String,
    pub last_seen: String,
    pub deployments: Vec<String>,
    pub metadata: HashMap<String, String>,
}

/// Response for deployment registration info
#[derive(Debug, Serialize, Deserialize)]
pub struct DeploymentResponse {
    pub tenant_id: String,
    pub deployment_key: String,
    pub created_at: String,
    pub last_seen: String,
    pub nodetype_version: Option<String>,
    pub node_count: Option<u64>,
}

/// Request to create a new tenant
#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    pub tenant_id: String,
    pub metadata: Option<HashMap<String, String>>,
}

/// Request to create a new deployment
#[derive(Debug, Deserialize)]
pub struct CreateDeploymentRequest {
    pub tenant_id: String,
    pub deployment_key: String,
}

/// List all registered tenants
pub async fn list_tenants(State(state): State<AppState>) -> Response {
    let storage = state.storage();
    let registry = storage.registry();

    match registry.list_tenants().await {
        Ok(tenants) => {
            let responses: Vec<TenantResponse> = tenants
                .into_iter()
                .map(|t| TenantResponse {
                    tenant_id: t.tenant_id,
                    created_at: t.created_at.to_rfc3339(),
                    last_seen: t.last_seen.to_rfc3339(),
                    deployments: t.deployments,
                    metadata: t.metadata,
                })
                .collect();

            Json(responses).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list tenants: {}", e),
        )
            .into_response(),
    }
}

/// Get a specific tenant
pub async fn get_tenant(State(state): State<AppState>, Path(tenant_id): Path<String>) -> Response {
    let storage = state.storage();
    let registry = storage.registry();

    match registry.get_tenant(&tenant_id).await {
        Ok(Some(tenant)) => Json(TenantResponse {
            tenant_id: tenant.tenant_id,
            created_at: tenant.created_at.to_rfc3339(),
            last_seen: tenant.last_seen.to_rfc3339(),
            deployments: tenant.deployments,
            metadata: tenant.metadata,
        })
        .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "Tenant not found").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get tenant: {}", e),
        )
            .into_response(),
    }
}

/// Register a new tenant
pub async fn create_tenant(
    State(state): State<AppState>,
    Json(req): Json<CreateTenantRequest>,
) -> Response {
    let storage = state.storage();
    let registry = storage.registry();

    let metadata = req.metadata.unwrap_or_default();

    match registry.register_tenant(&req.tenant_id, metadata).await {
        Ok(()) => (StatusCode::CREATED, "Tenant registered").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to register tenant: {}", e),
        )
            .into_response(),
    }
}

/// List deployments for a specific tenant (or all if no tenant specified)
pub async fn list_deployments(
    State(state): State<AppState>,
    tenant_id: Option<Path<String>>,
) -> Response {
    let storage = state.storage();
    let registry = storage.registry();

    let tenant_filter = tenant_id.as_ref().map(|p| p.as_str());

    match registry.list_deployments(tenant_filter).await {
        Ok(deployments) => {
            let responses: Vec<DeploymentResponse> = deployments
                .into_iter()
                .map(|d| DeploymentResponse {
                    tenant_id: d.tenant_id,
                    deployment_key: d.deployment_key,
                    created_at: d.created_at.to_rfc3339(),
                    last_seen: d.last_seen.to_rfc3339(),
                    nodetype_version: d.nodetype_version,
                    node_count: d.node_count,
                })
                .collect();

            Json(responses).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list deployments: {}", e),
        )
            .into_response(),
    }
}

/// Get a specific deployment
pub async fn get_deployment(
    State(state): State<AppState>,
    Path((tenant_id, deployment_key)): Path<(String, String)>,
) -> Response {
    let storage = state.storage();
    let registry = storage.registry();

    match registry.get_deployment(&tenant_id, &deployment_key).await {
        Ok(Some(deployment)) => Json(DeploymentResponse {
            tenant_id: deployment.tenant_id,
            deployment_key: deployment.deployment_key,
            created_at: deployment.created_at.to_rfc3339(),
            last_seen: deployment.last_seen.to_rfc3339(),
            nodetype_version: deployment.nodetype_version,
            node_count: deployment.node_count,
        })
        .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "Deployment not found").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get deployment: {}", e),
        )
            .into_response(),
    }
}

/// Register a new deployment
pub async fn create_deployment(
    State(state): State<AppState>,
    Json(req): Json<CreateDeploymentRequest>,
) -> Response {
    let storage = state.storage();
    let registry = storage.registry();

    match registry
        .register_deployment(&req.tenant_id, &req.deployment_key)
        .await
    {
        Ok(()) => (StatusCode::CREATED, "Deployment registered").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to register deployment: {}", e),
        )
            .into_response(),
    }
}
