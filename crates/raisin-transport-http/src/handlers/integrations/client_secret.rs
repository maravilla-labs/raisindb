// SPDX-License-Identifier: BSL-1.1

//! `oauth/client-secret` admin handler.
//!
//! Stores the OAuth *client* secret for an integration as
//! `client_secret_encrypted` (AES-256-GCM, base64 of the standard wire format).
//! The value is write-only: it is never returned by any read endpoint, and the
//! admin console renders only a "secret is set" indicator derived from the
//! presence of the encrypted property.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use base64::Engine as _;
use raisin_crypto::SecretBox;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use serde::{Deserialize, Serialize};

use super::require_admin;
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Request body for `oauth/client-secret`.
#[derive(Debug, Deserialize)]
pub struct SetClientSecretRequest {
    /// Path of the `raisin:Integration` node in the `raisin:system` workspace.
    pub integration_path: String,
    /// The new client secret. An empty string clears the stored secret,
    /// turning the integration back into a public client.
    pub client_secret: String,
}

/// Response for `oauth/client-secret`. Deliberately carries no secret material.
#[derive(Debug, Serialize)]
pub struct SetClientSecretResponse {
    pub ok: bool,
    /// Whether a secret is stored after this call.
    pub client_secret_set: bool,
}

/// Encrypt and store (or clear) an integration's OAuth client secret.
///
/// Admin-only. The plaintext secret exists only for the lifetime of this
/// request: it is encrypted before being written to the node and is never
/// logged, echoed, or persisted in cleartext.
pub async fn set_client_secret(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<SetClientSecretRequest>,
) -> Result<Json<SetClientSecretResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let svc = super::config_service(&state, &tenant.tenant_id, &repo, "integration-oauth");
    let mut node = svc
        .get_by_path(&req.integration_path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(req.integration_path.clone()))?;

    let client_secret_set = if req.client_secret.is_empty() {
        node.properties.remove("client_secret_encrypted");
        false
    } else {
        let master_key = state.get_master_key().map_err(|_| {
            ApiError::internal(
                "RAISIN_MASTER_KEY is not configured; cannot store an integration client secret",
            )
        })?;
        let ciphertext = SecretBox::new(&master_key)
            .encrypt(&req.client_secret)
            .map_err(|e| ApiError::internal(format!("failed to encrypt client secret: {e}")))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(ciphertext);
        node.properties.insert(
            "client_secret_encrypted".to_string(),
            PropertyValue::String(encoded),
        );
        true
    };

    svc.update_node(node).await?;

    Ok(Json(SetClientSecretResponse {
        ok: true,
        client_secret_set,
    }))
}
