// SPDX-License-Identifier: BSL-1.1

//! Tenant initialization middleware.
//!
//! Ensures NodeTypes are lazily initialized for a tenant on first request.

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use raisin_core::init::{calculate_nodetype_version, init_tenant_nodetypes};
use raisin_storage::{RegistryRepository, Storage};

use crate::state::AppState;

use super::types::TenantInfo;

/// Middleware that ensures tenant NodeTypes are initialized.
///
/// This middleware:
/// 1. Extracts tenant_id and deployment_key from request headers
/// 2. Checks if the tenant needs NodeType initialization
/// 3. Initializes NodeTypes if needed (lazy initialization on first request)
/// 4. Stores TenantInfo in request extensions for downstream handlers
///
/// Expected headers:
/// - `x-tenant-id`: Tenant identifier
/// - `x-deployment-key`: Deployment environment (e.g., "production", "staging")
///
/// If headers are missing, defaults to "default" tenant and "production" deployment.
pub async fn ensure_tenant_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let tenant_id = req
        .headers()
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("default")
        .to_string();

    let deployment_key = req
        .headers()
        .get("x-deployment-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("production")
        .to_string();

    // Check if tenant needs initialization
    let storage = state.storage();
    let current_version = calculate_nodetype_version();

    if let Ok(needs_init) = storage
        .registry()
        .get_deployment(&tenant_id, &deployment_key)
        .await
    {
        if needs_init.is_none()
            || needs_init.and_then(|d| d.nodetype_version).as_ref() != Some(&current_version)
        {
            tracing::info!(
                "Initializing NodeTypes for tenant {}/{}",
                tenant_id,
                deployment_key
            );

            if let Err(e) =
                init_tenant_nodetypes((**storage).clone(), &tenant_id, &deployment_key).await
            {
                tracing::warn!(
                    "NodeType initialization skipped for tenant {}/{}: {}",
                    tenant_id,
                    deployment_key,
                    e
                );
            }
        }
    } else {
        tracing::warn!(
            "Failed to check deployment status for {}/{}",
            tenant_id,
            deployment_key
        );
    }

    req.extensions_mut().insert(TenantInfo {
        tenant_id,
        deployment_key,
    });

    Ok(next.run(req).await)
}
