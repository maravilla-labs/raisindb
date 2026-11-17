// SPDX-License-Identifier: BSL-1.1

//! `POST /api/integrations/{repo}/oauth/start` — begin an outbound OAuth flow.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use serde::{Deserialize, Serialize};

use super::state_store::{now_secs, OAUTH_STATE_STORE, STATE_TTL_SECS};
use super::{is_connect_broker_url, json_str, oauth_config, require_admin, scopes_string};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Request body for `oauth/start`.
#[derive(Debug, Deserialize)]
pub struct StartRequest {
    /// Path of the `raisin:Integration` node in the `raisin:system` workspace.
    pub integration_path: String,
    /// Where to send the browser once the consent is over, instead of the
    /// admin console. Absolute `https` URL on an allow-listed origin
    /// (`[integrations] oauth_return_origins`); anything else is a 400 here,
    /// never a silent fallback. See `return_to.rs`.
    #[serde(default)]
    pub return_to: Option<String>,
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

    // Validated BEFORE anything is minted, so a refused return_to leaves no
    // half-started flow behind.
    let return_to = match req.return_to.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(rt) => Some(super::return_to::validate_configured(rt).map_err(|reason| {
            ApiError::new(
                axum::http::StatusCode::BAD_REQUEST,
                "RETURN_TO_NOT_ALLOWED",
                reason,
            )
            .with_field("return_to")
        })?),
    };

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
        // Two very different situations hide behind an empty client_id, and the
        // operator can act on neither with the generic message:
        //
        // - A MANAGED connector (consent brokered through Maravilla Connect)
        //   whose client the control plane simply has not minted yet. Expected
        //   and temporary; the console keys its "provisioning pending" surface
        //   off this code, so keep the code stable.
        // - A BYO connector genuinely missing its client id: a real
        //   misconfiguration the operator fixes in the form.
        if is_connect_broker_url(&auth_url) {
            ApiError::new(
                axum::http::StatusCode::BAD_REQUEST,
                "MANAGED_CONNECTOR_UNPROVISIONED",
                "This connector's OAuth client has not been provisioned yet by the \
                 Maravilla control plane. Provisioning runs out of band — retry shortly, \
                 or ask your Maravilla admin to run \"Repair\" for this organization.",
            )
        } else {
            ApiError::validation_failed("integration oauth_config.client_id is missing")
        }
    })?;
    // Derived, not trusted: a stored value that does not point at the one route
    // `oauth_callback` is registered on is repaired here, so consent never ends
    // on a 404. `oauth_callback` resolves it the same way — the provider
    // requires the authorize and token requests to match byte-for-byte.
    let redirect_uri = super::resolve_redirect_uri(&cfg, &repo);
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
    if let Some(rt) = return_to {
        OAUTH_STATE_STORE.set_return_to(&state_token, rt);
    }

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
