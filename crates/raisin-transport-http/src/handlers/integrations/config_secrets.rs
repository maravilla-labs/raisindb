// SPDX-License-Identifier: BSL-1.1

//! `PUT /api/integrations/{repo}/config-secrets` — write connector-level secrets.
//!
//! Generalizes the single-purpose `oauth/client-secret` endpoint to the whole
//! set of connector-level properties a config schema flags `meta.secret: true`.
//! Values are merged into one AES-256-GCM blob (`config_secrets_encrypted`), and
//! the field **names** are mirrored in plaintext (`config_secret_fields`) so the
//! console can render an "is set" indicator without ever decrypting.
//!
//! Write-only, like its predecessor: no read endpoint returns a value, and a
//! response never echoes one back.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_crypto::SecretBox;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::require_admin;
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// The legacy dedicated property. Kept in sync so the OAuth callback and the
/// token-refresh job — which both read it directly — need no change, and so a
/// rollback still finds the secret where it expects it.
const LEGACY_CLIENT_SECRET_FIELD: &str = "client_secret";

/// Request body for `config-secrets`.
#[derive(Debug, Deserialize)]
pub struct SetConfigSecretsRequest {
    /// Path of the `raisin:Integration` node in the `raisin:system` workspace.
    pub integration_path: String,
    /// Field → value. A `null` value **clears** that field; a field simply not
    /// mentioned is left untouched, so a form can submit only what changed and
    /// never has to round-trip a secret it was not shown.
    pub secrets: BTreeMap<String, Option<String>>,
}

/// Response — deliberately carries no secret material.
#[derive(Debug, Serialize)]
pub struct SetConfigSecretsResponse {
    pub ok: bool,
    /// Names of the fields stored after this call.
    pub secret_fields_set: Vec<String>,
}

/// Merge connector-level secrets into the encrypted blob.
pub async fn set_config_secrets(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<SetConfigSecretsRequest>,
) -> Result<Json<SetConfigSecretsResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let master_key = state.get_master_key().map_err(|_| {
        ApiError::internal("RAISIN_MASTER_KEY is not configured; cannot store connector secrets")
    })?;
    let secret_box = SecretBox::new(&master_key);

    let svc = super::config_service(&state, &tenant.tenant_id, &repo, "integration-secrets");
    let mut node = svc
        .get_by_path(&req.integration_path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(req.integration_path.clone()))?;

    // Decrypt the existing map so unmentioned fields survive. A blob we cannot
    // decrypt (wrong/rotated key) is treated as empty rather than failing —
    // otherwise the connector would be permanently unfixable through the API.
    let mut current: Map<String, Value> = node
        .properties
        .get("config_secrets_encrypted")
        .and_then(|pv| match pv {
            PropertyValue::String(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        })
        .and_then(|enc| secret_box.decrypt_json(&enc).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();

    for (field, value) in req.secrets {
        match value {
            Some(v) if !v.is_empty() => {
                current.insert(field, Value::String(v));
            }
            // Explicit null or empty string clears.
            _ => {
                current.remove(&field);
            }
        }
    }

    let mut field_names: Vec<String> = current.keys().cloned().collect();
    field_names.sort();

    if current.is_empty() {
        node.properties.remove("config_secrets_encrypted");
        node.properties.remove("config_secret_fields");
    } else {
        let encoded = secret_box
            .encrypt_json(&Value::Object(current.clone()))
            .map_err(|e| ApiError::internal(format!("failed to encrypt connector secrets: {e}")))?;
        node.properties.insert(
            "config_secrets_encrypted".to_string(),
            PropertyValue::String(encoded),
        );
        let names = serde_json::to_value(&field_names)
            .ok()
            .and_then(|v| serde_json::from_value::<PropertyValue>(v).ok())
            .ok_or_else(|| ApiError::internal("failed to encode secret field names"))?;
        node.properties
            .insert("config_secret_fields".to_string(), names);
    }

    // Mirror `client_secret` into the legacy property. The OAuth callback and
    // the refresh job read `client_secret_encrypted` directly; keeping both in
    // step means neither needs to change now, and a rollback still works.
    sync_legacy_client_secret(
        &mut node,
        current.get(LEGACY_CLIENT_SECRET_FIELD),
        &secret_box,
    )?;

    svc.update_node(node).await?;

    Ok(Json(SetConfigSecretsResponse {
        ok: true,
        secret_fields_set: field_names,
    }))
}

/// Keep `client_secret_encrypted` in step with the `client_secret` entry of the
/// generic map (writing it separately, since the two use different encodings:
/// the legacy property is base64 of the raw wire format, not of a JSON blob).
fn sync_legacy_client_secret(
    node: &mut raisin_models::nodes::Node,
    value: Option<&Value>,
    secret_box: &SecretBox,
) -> Result<(), ApiError> {
    use base64::Engine as _;
    match value.and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        Some(plain) => {
            let ciphertext = secret_box
                .encrypt(plain)
                .map_err(|e| ApiError::internal(format!("failed to encrypt client secret: {e}")))?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(ciphertext);
            node.properties.insert(
                "client_secret_encrypted".to_string(),
                PropertyValue::String(encoded),
            );
        }
        None => {
            node.properties.remove("client_secret_encrypted");
        }
    }
    Ok(())
}
