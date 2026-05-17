//! Superadmin handlers for tenant lifecycle (provision + delete).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use raisin_core::init::init_tenant_nodetypes;
use raisin_models::admin_user::AdminAccessFlags;
use raisin_transport_http::state::AppState;
use serde::{Deserialize, Serialize};

use crate::management::types::ApiResponse;

const DEFAULT_DEPLOYMENT_KEY: &str = "production";
const ADMIN_USERNAME: &str = "admin";

#[derive(Debug, Deserialize)]
pub struct ProvisionTenantRequest {
    pub tenant_id: String,
    #[serde(default)]
    pub admin_password: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProvisionTenantResponse {
    pub tenant_id: String,
    pub admin_username: String,
    pub admin_password: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Provision a new tenant.
///
/// Creates an `admin` user (with generated password if none supplied) and
/// triggers NodeType initialization for the tenant.
/// Returns 409 if the tenant already has an admin user.
pub async fn provision_tenant(
    State(app_state): State<AppState>,
    Json(req): Json<ProvisionTenantRequest>,
) -> Result<(StatusCode, Json<ApiResponse<ProvisionTenantResponse>>), StatusCode> {
    let auth = app_state
        .auth_service()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .clone();

    // Reject if tenant already has any admin users.
    let has_users = auth
        .has_users(&req.tenant_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if has_users {
        tracing::warn!(
            tenant_id = %req.tenant_id,
            "Tenant already exists; refusing to provision"
        );
        return Err(StatusCode::CONFLICT);
    }

    let password = req
        .admin_password
        .clone()
        .unwrap_or_else(|| nanoid::nanoid!(24));

    let _user = auth
        .create_superadmin_with_password(
            req.tenant_id.clone(),
            ADMIN_USERNAME.to_string(),
            password.clone(),
        )
        .map_err(|e| {
            tracing::error!(
                tenant_id = %req.tenant_id,
                error = %e,
                "Failed to create admin user for new tenant"
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Lazy NodeType init — best-effort.
    let storage = app_state.storage();
    if let Err(e) =
        init_tenant_nodetypes((**storage).clone(), &req.tenant_id, DEFAULT_DEPLOYMENT_KEY).await
    {
        tracing::warn!(
            tenant_id = %req.tenant_id,
            error = %e,
            "NodeType init skipped during tenant provisioning"
        );
    }

    let _ = AdminAccessFlags::default(); // type kept used so the import line stays.

    tracing::warn!(
        tenant_id = %req.tenant_id,
        "Provisioned new tenant via superadmin"
    );

    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(ProvisionTenantResponse {
            tenant_id: req.tenant_id,
            admin_username: ADMIN_USERNAME.to_string(),
            admin_password: password,
            created_at: chrono::Utc::now(),
        })),
    ))
}

#[derive(Debug, Serialize)]
pub struct DeleteTenantResponse {
    pub tenant_id: String,
    pub deleted: bool,
    pub cfs_wiped: usize,
    pub compacted: bool,
    /// CFs that failed to wipe (best-effort; surfaced for operator visibility).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped_cfs: Vec<String>,
}

/// Wipe everything for a tenant.
///
/// Performs a structural data wipe by range-deleting every tenant-prefixed
/// key across the RocksDB CFs that hold tenant data, then triggers compaction
/// over the wiped ranges (see `RocksDBStorage::delete_tenant_data`).
/// Admin users for the tenant are also removed.
///
/// Idempotent — re-running on an already-empty tenant succeeds with the same
/// shape.
///
/// What's intentionally not touched:
/// - Cluster replication state (`OPERATION_LOG`, `APPLIED_OPS`) — managed by
///   replication GC; deleting it here would desync peers.
/// - Tantivy / HNSW index directories on disk — tenant-isolated per directory,
///   so leftover files are not a security issue. Operators can sweep them
///   separately if disk pressure matters.
pub async fn delete_tenant(
    State(app_state): State<AppState>,
    Path(tenant_id): Path<String>,
) -> Result<Json<ApiResponse<DeleteTenantResponse>>, StatusCode> {
    tracing::warn!(tenant_id = %tenant_id, "Superadmin: deleting tenant");

    // 1. Delete admin users for this tenant (best-effort; the structural
    //    ADMIN_USERS wipe in step 3 also clears these, but going through the
    //    auth service triggers any operation-capture hooks the cluster needs).
    if let Some(auth) = app_state.auth_service() {
        match auth.list_users(&tenant_id) {
            Ok(users) => {
                for u in users {
                    if let Err(e) = auth.delete_user(&tenant_id, &u.username) {
                        tracing::warn!(
                            tenant_id = %tenant_id,
                            username = %u.username,
                            error = %e,
                            "Failed to delete admin user via auth service"
                        );
                    }
                }
            }
            Err(e) => tracing::warn!(
                tenant_id = %tenant_id,
                error = %e,
                "Failed to list admin users for deletion"
            ),
        }
    }

    // 2. Structural data wipe across every tenant-prefixed CF.
    let storage = app_state.storage().clone();
    let report = match storage.delete_tenant_data(&tenant_id) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(
                tenant_id = %tenant_id,
                error = %e,
                "Tenant data wipe failed"
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    tracing::warn!(
        tenant_id = %tenant_id,
        cfs_wiped = report.cfs_wiped,
        "Tenant data fully wiped from RocksDB CFs"
    );

    Ok(Json(ApiResponse::ok(DeleteTenantResponse {
        tenant_id,
        deleted: true,
        cfs_wiped: report.cfs_wiped,
        compacted: report.compacted,
        skipped_cfs: report.skipped,
    })))
}
