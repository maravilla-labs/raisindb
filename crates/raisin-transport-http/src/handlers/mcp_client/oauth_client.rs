// SPDX-License-Identifier: BSL-1.1

//! Supplying an OAuth client by hand.
//!
//! Dynamic registration covers the servers that support it, but two cases need
//! an operator to bring their own client, and until now neither had anywhere to
//! put one:
//!
//! - **The server has no registration endpoint.** The docs told the operator to
//!   register RaisinDB themselves and gave them the redirect URI — then offered
//!   no field for the resulting client id.
//! - **The server requires client authentication.** Registration may issue a
//!   PUBLIC client (or the operator may have registered a confidential app
//!   themselves), and then the token exchange is rejected for want of a secret.
//!   Microsoft Entra answers `AADSTS7000218` here.
//!
//! Write-only, exactly like the static credential in [`super::credential`]: the
//! secret is encrypted on arrival and no endpoint ever returns it. Reads expose
//! only whether one is set.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_crypto::SecretBox;
use raisin_models::auth::AuthContext;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{connection_service, load_connection, require_admin, set_prop};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

const ACTOR: &str = "mcp-connection-oauth-client";

/// An operator-supplied OAuth client.
#[derive(Debug, Deserialize)]
pub struct OauthClientRequest {
    /// Client identifier issued by the provider. Required.
    pub client_id: String,
    /// Client secret, for a confidential client. Encrypted immediately.
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Issuer, when `discover` has not already found one.
    #[serde(default)]
    pub issuer: Option<String>,
    /// Authorization endpoint, when not already discovered.
    #[serde(default)]
    pub authorization_endpoint: Option<String>,
    /// Token endpoint, when not already discovered.
    #[serde(default)]
    pub token_endpoint: Option<String>,
    /// Scopes to request, when not already discovered.
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
}

/// `PUT /api/mcp-connections/{repo}/{slug}/oauth/client`
///
/// Merges onto whatever `discover` already found, so an operator who only needs
/// to add a secret does not have to re-type the endpoints.
pub async fn set_oauth_client(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<OauthClientRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    if req.client_id.trim().is_empty() {
        return Err(ApiError::validation_failed("client_id is required"));
    }

    let (mut node, _) = load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;

    let mut cfg = node
        .properties
        .get("oauth_client")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or_else(|| json!({}));
    if !cfg.is_object() {
        cfg = json!({});
    }
    let object = cfg.as_object_mut().expect("just ensured an object");

    object.insert("client_id".into(), json!(req.client_id.trim()));
    for (key, value) in [
        ("issuer", req.issuer),
        ("authorization_endpoint", req.authorization_endpoint),
        ("token_endpoint", req.token_endpoint),
    ] {
        // Only overwrite what was actually supplied — an omitted field must not
        // blank out something discovery already established.
        if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
            object.insert(key.into(), json!(value.trim()));
        }
    }
    if let Some(scopes) = req.scopes {
        object.insert("scopes".into(), json!(scopes));
    }
    object.insert("configured_by_operator".into(), json!(true));
    object.insert(
        "configured_at".into(),
        json!(chrono::Utc::now().to_rfc3339()),
    );

    set_prop(&mut node, "oauth_client", cfg)?;
    set_prop(&mut node, "auth_kind", json!("oauth"))?;

    let mut secret_set = super::str_prop(&node, "oauth_client_secret_encrypted").is_some();
    if let Some(secret) = req.client_secret.filter(|s| !s.is_empty()) {
        let master_key = state.get_master_key().map_err(|_| {
            ApiError::internal("RAISIN_MASTER_KEY is not configured; cannot store a client secret")
        })?;
        let blob = SecretBox::new(&master_key)
            .encrypt_json(&json!({ "value": secret }))
            .map_err(|e| ApiError::internal(format!("failed to encrypt the client secret: {e}")))?;
        set_prop(&mut node, "oauth_client_secret_encrypted", json!(blob))?;
        secret_set = true;
    }

    connection_service(&state, &tenant.tenant_id, &repo, ACTOR)
        .update_node(node)
        .await?;

    // Never the secret, and never the client_id echoed back as if it were a
    // read API — this endpoint confirms, it does not disclose.
    Ok(Json(json!({
        "ok": true,
        "client_id_set": true,
        "client_secret_set": secret_set,
    })))
}

/// `DELETE /api/mcp-connections/{repo}/{slug}/oauth/client`
///
/// Clears the operator-supplied client. Tokens go too: they were issued to that
/// client and are worthless without it, and leaving them would show a connection
/// as authorized when nothing can refresh it.
pub async fn clear_oauth_client(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    let (mut node, _) = load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;
    node.properties.remove("oauth_client");
    node.properties.remove("oauth_client_secret_encrypted");
    node.properties.remove("tokens_encrypted");
    node.properties.remove("expires_at");

    connection_service(&state, &tenant.tenant_id, &repo, ACTOR)
        .update_node(node)
        .await?;

    Ok(Json(json!({
        "ok": true,
        "client_id_set": false,
        "client_secret_set": false,
        "oauth_connected": false,
    })))
}
