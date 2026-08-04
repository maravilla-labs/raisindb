// SPDX-License-Identifier: BSL-1.1

//! Write-only static credential for an MCP connection.
//!
//! Same contract as `integrations::set_config_secrets`: the value goes in, and
//! nothing ever brings it back out. No read endpoint returns it, the response
//! does not echo it, and the console renders only an "is set" indicator built
//! from `credential_set` in the redacted view.
//!
//! Setting a credential also sets `auth_kind: "static"` when the connection was
//! `none`, because a stored credential that the auth mode never applies is a
//! silent no-op — the operator would see "credential set" and still get 401s.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_crypto::SecretBox;
use raisin_mcp::client::AuthKind;
use raisin_models::auth::AuthContext;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{connection_service, load_connection, require_admin, set_prop};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

const ACTOR: &str = "mcp-connection-credential";

/// Request body for setting a credential.
#[derive(Debug, Deserialize)]
pub struct SetCredentialRequest {
    /// The secret itself — a bearer token or API key.
    pub value: String,
    /// Optional placement override: `{ scheme: "bearer" }` or
    /// `{ scheme: "header", header_name: "X-Api-Key" }`.
    #[serde(default)]
    pub static_auth: Option<serde_json::Value>,
}

/// Response — carries no secret material.
#[derive(Debug, Serialize)]
pub struct CredentialResponse {
    /// Always true on success.
    pub ok: bool,
    /// Whether a credential is stored after this call.
    pub credential_set: bool,
    /// The auth mode in effect after this call.
    pub auth_kind: AuthKind,
}

/// `PUT /api/mcp-connections/{repo}/{slug}/credential`
pub async fn set_credential(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<SetCredentialRequest>,
) -> Result<Json<CredentialResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    if req.value.is_empty() {
        return Err(ApiError::validation_failed(
            "credential value must not be empty; use DELETE to clear it",
        ));
    }

    let master_key = state.get_master_key().map_err(|_| {
        ApiError::internal("RAISIN_MASTER_KEY is not configured; cannot store MCP credentials")
    })?;

    let (mut node, descriptor) =
        load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;

    let ciphertext = SecretBox::new(&master_key)
        .encrypt_json(&json!({ "value": req.value }))
        .map_err(|e| ApiError::internal(format!("failed to encrypt MCP credential: {e}")))?;
    set_prop(&mut node, "credential_encrypted", json!(ciphertext))?;

    if let Some(static_auth) = req.static_auth {
        set_prop(&mut node, "static_auth", static_auth)?;
    }

    // A credential stored under auth_kind "none" would never be sent.
    let auth_kind = match descriptor.auth_kind {
        AuthKind::None => {
            set_prop(&mut node, "auth_kind", json!("static"))?;
            AuthKind::Static
        }
        existing => existing,
    };

    connection_service(&state, &tenant.tenant_id, &repo, ACTOR)
        .update_node(node)
        .await?;

    Ok(Json(CredentialResponse {
        ok: true,
        credential_set: true,
        auth_kind,
    }))
}

/// `DELETE /api/mcp-connections/{repo}/{slug}/credential`
pub async fn clear_credential(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<CredentialResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let (mut node, descriptor) =
        load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;

    node.properties.remove("credential_encrypted");

    // Clearing the credential of a static connection leaves it unauthenticated;
    // say so in the stored state rather than leaving a mode that cannot work.
    let auth_kind = match descriptor.auth_kind {
        AuthKind::Static => {
            set_prop(&mut node, "auth_kind", json!("none"))?;
            AuthKind::None
        }
        existing => existing,
    };

    connection_service(&state, &tenant.tenant_id, &repo, ACTOR)
        .update_node(node)
        .await?;

    Ok(Json(CredentialResponse {
        ok: true,
        credential_set: false,
        auth_kind,
    }))
}
