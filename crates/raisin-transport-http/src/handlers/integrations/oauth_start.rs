// SPDX-License-Identifier: BSL-1.1

//! `POST /api/integrations/{repo}/oauth/start` — begin an outbound OAuth flow.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use serde::{Deserialize, Serialize};

use super::state_store::{now_secs, OAUTH_STATE_STORE, STATE_TTL_SECS};
use super::{json_str, oauth_config, require_admin, scopes_string};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Request body for `oauth/start`.
#[derive(Debug, Deserialize)]
pub struct StartRequest {
    /// Path of the `raisin:Integration` node in the `raisin:system` workspace.
    pub integration_path: String,
}

/// Response for `oauth/start`.
#[derive(Debug, Serialize)]
pub struct StartResponse {
    /// The provider authorization URL the browser should be sent to.
    pub auth_url: String,
    /// Opaque CSRF/correlation token echoed back on the callback.
    pub state: String,
}

/// Mint an authorization URL and a single-use `state` for connecting an account.
pub async fn start(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<StartRequest>,
) -> Result<Json<StartResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let svc = super::config_service(&state, &tenant.tenant_id, &repo, "integration-oauth");
    let node = svc
        .get_by_path(&req.integration_path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(req.integration_path.clone()))?;

    let cfg = oauth_config(&node);
    let auth_url = json_str(&cfg, "auth_url").ok_or_else(|| {
        ApiError::validation_failed("integration oauth_config.auth_url is missing")
    })?;
    let client_id = json_str(&cfg, "client_id").ok_or_else(|| {
        ApiError::validation_failed("integration oauth_config.client_id is missing")
    })?;
    let redirect_uri = json_str(&cfg, "redirect_uri").ok_or_else(|| {
        ApiError::validation_failed("integration oauth_config.redirect_uri is missing")
    })?;
    let scopes = scopes_string(&cfg);

    let state_token = nanoid::nanoid!();
    OAUTH_STATE_STORE.insert(
        state_token.clone(),
        tenant.tenant_id.clone(),
        repo.clone(),
        req.integration_path.clone(),
        now_secs(),
        STATE_TTL_SECS,
    );

    // `access_type=offline` + `prompt=consent` request a refresh token on every
    // connect (Google semantics; harmless for providers that ignore them).
    let params = [
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("scope", scopes.as_str()),
        ("state", state_token.as_str()),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ];
    let url = reqwest::Url::parse_with_params(&auth_url, &params)
        .map_err(|e| ApiError::validation_failed(format!("invalid auth_url: {e}")))?;

    Ok(Json(StartResponse {
        auth_url: url.to_string(),
        state: state_token,
    }))
}
