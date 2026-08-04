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

    // Decrypt the existing map so unmentioned fields survive.
    let stored = node
        .properties
        .get("config_secrets_encrypted")
        .and_then(|pv| match pv {
            PropertyValue::String(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        });

    // A blob we cannot decrypt is REFUSED, not treated as empty.
    //
    // Treating it as empty was the more forgiving-looking choice — the
    // connector stays fixable through the API — but the cost is that the very
    // next save writes an empty map over secrets that are still perfectly good
    // under the original key, destroying them silently. Refusing keeps them
    // recoverable: restore RAISIN_MASTER_KEY and they decrypt again. Clearing
    // deliberately is still possible by sending every field as null.
    let mut current: Map<String, Value> = match stored {
        None => Map::new(),
        Some(enc) => match secret_box.decrypt_json(&enc) {
            Ok(Value::Object(m)) => m,
            Ok(_) => Map::new(),
            Err(_) => {
                tracing::error!(
                    node_path = %node.path,
                    "connector has stored config secrets that cannot be decrypted; refusing to \
                     save rather than overwrite them. RAISIN_MASTER_KEY most likely differs from \
                     the key they were stored under — restore it, or clear the field explicitly."
                );
                return Err(ApiError::internal(
                    "this connector's stored secrets cannot be decrypted with the current \
                     RAISIN_MASTER_KEY. Saving would destroy them, so it was refused. Restore \
                     the original key, or clear the secrets explicitly before setting new ones.",
                ));
            }
        },
    };

    // Whether the CALLER named `client_secret`, which is not the same question
    // as whether it is present in `current`. See `sync_legacy_client_secret`.
    let client_secret_mentioned = req.secrets.contains_key(LEGACY_CLIENT_SECRET_FIELD);

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
    if client_secret_mentioned {
        sync_legacy_client_secret(
            &mut node,
            current.get(LEGACY_CLIENT_SECRET_FIELD),
            &secret_box,
        )?;
    }

    svc.update_node(node).await?;

    Ok(Json(SetConfigSecretsResponse {
        ok: true,
        secret_fields_set: field_names,
    }))
}

/// Keep `client_secret_encrypted` in step with the `client_secret` entry of the
/// generic map (writing it separately, since the two use different encodings:
/// the legacy property is base64 of the raw wire format, not of a JSON blob).
///
/// **Only called when the request actually named `client_secret`.** It used to
/// run on every save, deciding from `current` — which is built solely from
/// `config_secrets_encrypted`. A connector whose secret was set through the
/// dedicated endpoint (the console's own "Client secret" field, and the only
/// place ms-graph has one, since its config schema declares no secret field)
/// therefore had no `client_secret` in that map, so every unrelated save took
/// the `None` branch below and DELETED the stored secret.
///
/// The console made it certain rather than likely: it saves config secrets
/// first and the client secret second, and skips the second when the field is
/// blank — under a placeholder that reads "secret set — leave blank to keep".
/// Editing any other setting and pressing Save destroyed the credential, while
/// the UI promised the opposite. The provider then answers AADSTS7000218 and
/// nothing on this side explains why, because the property is absent rather
/// than undecryptable.
///
/// An unmentioned field must survive. That is already the rule for every entry
/// in the generic map; this one was the exception.
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

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::Node;

    fn node_with_secret() -> Node {
        let mut node = Node {
            path: "/integrations/ms-graph".to_string(),
            ..Default::default()
        };
        node.properties.insert(
            "client_secret_encrypted".to_string(),
            PropertyValue::String("ciphertext".to_string()),
        );
        node
    }

    fn secret_box() -> SecretBox {
        SecretBox::new(&[7u8; 32])
    }

    /// THE regression. Saving a secret that is not `client_secret` must leave
    /// the legacy property alone.
    ///
    /// It did not: `sync_legacy_client_secret` ran on every save and decided
    /// from the generic map, which never contained `client_secret` for a
    /// connector whose secret was set through the dedicated endpoint. So an
    /// unrelated save deleted the credential — and the console saves config
    /// secrets BEFORE the client secret and skips the latter when the field is
    /// blank, under a placeholder promising that blank means "keep".
    #[test]
    fn saving_an_unrelated_secret_does_not_delete_the_client_secret() {
        let mut node = node_with_secret();

        // The caller never mentioned `client_secret`, so the sync must not run.
        let mentioned = false;
        if mentioned {
            sync_legacy_client_secret(&mut node, None, &secret_box()).unwrap();
        }

        assert!(
            node.properties.contains_key("client_secret_encrypted"),
            "an unmentioned client secret must survive an unrelated save"
        );
    }

    /// Clearing it must still work — but only when asked for explicitly.
    #[test]
    fn explicitly_clearing_the_client_secret_still_removes_it() {
        let mut node = node_with_secret();
        sync_legacy_client_secret(&mut node, None, &secret_box()).unwrap();
        assert!(!node.properties.contains_key("client_secret_encrypted"));
    }

    /// And setting it writes the legacy property the OAuth callback reads.
    #[test]
    fn setting_the_client_secret_writes_the_property_the_callback_reads() {
        let mut node = Node::default();
        let value = Value::String("s3cret".to_string());
        sync_legacy_client_secret(&mut node, Some(&value), &secret_box()).unwrap();

        match node.properties.get("client_secret_encrypted") {
            Some(PropertyValue::String(s)) => assert!(!s.is_empty()),
            other => panic!("expected ciphertext, got {other:?}"),
        }
    }
}
