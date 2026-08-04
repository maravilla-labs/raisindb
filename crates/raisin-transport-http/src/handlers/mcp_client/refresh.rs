// SPDX-License-Identifier: BSL-1.1

//! `POST /api/mcp-connections/{repo}/{slug}/refresh-tools` — discover on demand.
//!
//! Enqueues the same `McpToolDiscovery` job the periodic scan uses; there is no
//! second discovery path. The handler serializes a manual run against a
//! scheduled one with a `KeyedMutex`, so an operator pressing "Refresh" while a
//! scheduled run is in flight waits for it rather than being rejected.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use serde_json::{json, Value};

use super::{load_connection, require_admin};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

const ACTOR: &str = "mcp-connection-refresh";

/// `POST /api/mcp-connections/{repo}/{slug}/refresh-tools`
pub async fn refresh_tools(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    // Load first so a bad slug is a 404 rather than a job that quietly no-ops.
    let (_, descriptor) = load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;
    if !descriptor.is_callable() {
        return Err(ApiError::validation_failed(format!(
            "connection `{slug}` is disabled; enable it before refreshing tools"
        )));
    }

    let job_id = enqueue_discovery(&state, &tenant.tenant_id, &repo, &slug).await?;
    Ok(Json(json!({ "ok": true, "job_id": job_id })))
}

/// Enqueue a discovery job for one connection, returning its job id.
///
/// Shared with the tool-toggle endpoint so both go through one enqueue path.
#[cfg(feature = "storage-rocksdb")]
pub(crate) async fn enqueue_discovery(
    state: &AppState,
    tenant: &str,
    repo: &str,
    slug: &str,
) -> Result<String, ApiError> {
    use raisin_storage::jobs::{JobContext, JobId, JobType};

    let job_type = JobType::McpToolDiscovery {
        connection_slug: slug.to_string(),
        tenant_id: tenant.to_string(),
        repo_id: repo.to_string(),
        // Both callers of this helper are operator actions: the refresh button
        // and the per-tool enable toggle.
        source: raisin_storage::jobs::McpDiscoverySource::Manual,
    };
    let job_id = JobId::new();
    let context = JobContext {
        tenant_id: tenant.to_string(),
        repo_id: repo.to_string(),
        branch: super::CONFIG_BRANCH.to_string(),
        workspace_id: super::CONFIG_WORKSPACE.to_string(),
        revision: raisin_hlc::HLC::now(),
        metadata: std::collections::HashMap::new(),
    };

    // Context before registration: the dispatcher may pick the job up the
    // instant it is registered.
    state
        .storage
        .job_data_store()
        .put(&job_id, &context)
        .map_err(|e| ApiError::internal(format!("failed to store discovery job context: {e}")))?;

    let id = job_id.to_string();
    state
        .storage
        .job_registry()
        .register_job_with_id_idempotent(
            job_id,
            job_type,
            tenant.to_string(),
            format!("mcp-discovery:{tenant}:{repo}:{slug}"),
            None,
        )
        .await
        .map_err(|e| ApiError::internal(format!("failed to enqueue discovery job: {e}")))?;

    Ok(id)
}
