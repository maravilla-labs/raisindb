// SPDX-License-Identifier: BSL-1.1

//! `/api/integrations/{repo}/connections` — manage a connector's connections.
//!
//! A *connection* is one entry in `raisin:Integration.connected_accounts`. Until
//! now the only way to create one was completing an OAuth flow, so a connector
//! authenticating with a username and password (IMAP) had no reachable path at
//! all — the adapter read `credential.password`, but nothing could ever write
//! it. These endpoints supply that path, and in doing so also give every
//! connector the ability to hold **several** connections (two Entra tenants, two
//! mailboxes) rather than one.
//!
//! Secrets follow the same write-only rule as everywhere else in this module:
//! values arrive under `secrets`, are encrypted into `secrets_encrypted`, and no
//! response ever contains ciphertext or plaintext. Reading connections through
//! *this* endpoint rather than off the raw node is what keeps `tokens_encrypted`
//! out of the browser.

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use raisin_crypto::SecretBox;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::integrations::ConnectedAccount;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use super::accounts_lock::with_accounts_lock;
use super::require_admin;
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Safe projection of a connection. Never carries ciphertext or plaintext
/// secrets — only whether each secret field is set.
#[derive(Debug, Serialize)]
pub struct ConnectionView {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub auth_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// Per-connection non-secret configuration.
    pub config: Value,
    /// Names of the secret fields that are stored.
    pub secret_fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Scopes the provider granted when this account last authorized.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// Scopes the connector NOW requests that this account has not granted.
    ///
    /// Non-empty means the account is healthy in every observable way — the
    /// token refreshes, reads work — and yet cannot perform some operation,
    /// because neither Microsoft nor Google upgrades an existing grant. Only a
    /// fresh authorization does. Surfacing it here is what turns a 403 at write
    /// time into something an operator can see and act on beforehand.
    ///
    /// Empty when the account reported no scopes at all: the provider not
    /// saying what it granted is not evidence that it granted nothing, and
    /// showing every such account as broken would train people to ignore this.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_scopes: Vec<String>,
}

impl From<&ConnectedAccount> for ConnectionView {
    fn from(a: &ConnectedAccount) -> Self {
        Self {
            id: a.id.clone(),
            label: a.display_label().to_string(),
            subject: a.subject.clone(),
            auth_kind: match a.auth_kind {
                raisin_models::nodes::integrations::AuthKind::Oauth => "oauth".into(),
                raisin_models::nodes::integrations::AuthKind::Config => "config".into(),
            },
            expires_at: a.expires_at,
            config: a.config_or_empty(),
            secret_fields: a.secret_fields.clone(),
            created_at: a.created_at.clone(),
            scopes: a.scopes.clone(),
            // Filled in by the caller, which is the only place that can see the
            // connector's current scope list.
            missing_scopes: Vec::new(),
        }
    }
}

/// Which of the connector's currently-requested scopes an account has not
/// granted.
///
/// Comparison is exact and case-sensitive: providers echo scopes back verbatim,
/// and normalizing would risk calling a genuinely missing scope present.
///
/// Returns nothing when the account reported no scopes — see
/// [`ConnectionView::missing_scopes`].
fn missing_scopes(granted: &[String], requested: &str) -> Vec<String> {
    if granted.is_empty() {
        return Vec::new();
    }
    requested
        .split_whitespace()
        .filter(|want| !granted.iter().any(|has| has == want))
        .map(str::to_string)
        .collect()
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub integration_path: String,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub connections: Vec<ConnectionView>,
}

/// List a connector's connections, secrets redacted.
pub async fn list_connections(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    require_admin(auth.as_deref())?;
    let svc = super::config_service(&state, &tenant.tenant_id, &repo, "integration-connections");
    let node = svc
        .get_by_path(&q.integration_path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(q.integration_path.clone()))?;

    let accounts = parse_accounts(&node);
    // The connector's CURRENT ask, which is what a granted scope has to be
    // measured against — the list can be widened by a package update long after
    // an account consented.
    let requested = super::scopes_string(&super::oauth_config(&node));
    Ok(Json(ListResponse {
        connections: accounts
            .iter()
            .map(|a| {
                let mut view = ConnectionView::from(a);
                view.missing_scopes = missing_scopes(&a.scopes, &requested);
                view
            })
            .collect(),
    }))
}

/// Body for creating or updating a connection.
#[derive(Debug, Deserialize)]
pub struct UpsertConnectionRequest {
    pub integration_path: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Non-secret per-connection configuration (host/port/tenant_id/username).
    #[serde(default)]
    pub config: Option<Value>,
    /// Secret fields. `null` clears; an omitted field is left unchanged on
    /// update, so an edit form never has to round-trip a value it was not shown.
    #[serde(default)]
    pub secrets: BTreeMap<String, Option<String>>,
}

/// Create a credential-based connection.
///
/// This is the non-OAuth counterpart to `oauth/callback`: it is what makes an
/// IMAP mailbox connectable from the console.
pub async fn create_connection(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<UpsertConnectionRequest>,
) -> Result<Json<ConnectionView>, ApiError> {
    require_admin(auth.as_deref())?;
    let secret_box = SecretBox::new(&state.get_master_key().map_err(|_| {
        ApiError::internal("RAISIN_MASTER_KEY is not configured; cannot store connection secrets")
    })?);

    let config = reject_secrets_in_config(req.config.clone(), &req.secrets)?;
    let id = nanoid::nanoid!();

    let view = with_accounts_lock(
        &state,
        &tenant.tenant_id,
        &repo,
        &req.integration_path,
        "integration-connections",
        move |node| {
            let mut accounts = parse_accounts(node);
            let provider_type = super::json_str(
                &serde_json::to_value(&node.properties).unwrap_or(Value::Null),
                "provider_type",
            );

            let secrets = encrypt_secrets(&Map::new(), &req.secrets, &secret_box)?;
            let account = ConnectedAccount {
                id: id.clone(),
                label: req.label.clone().filter(|s| !s.is_empty()),
                subject: None,
                provider_type,
                auth_kind: raisin_models::nodes::integrations::AuthKind::Config,
                expires_at: None,
                tokens_encrypted: None,
                config: Some(config.clone()),
                secrets_encrypted: secrets.blob,
                secret_fields: secrets.field_names,
                created_at: Some(chrono::Utc::now().to_rfc3339()),
                // A credential connection has no OAuth grant, so it has no
                // scopes and can never report a shortfall.
                scopes: Vec::new(),
            };

            let view = ConnectionView::from(&account);
            accounts.push(account);
            write_accounts(node, accounts)?;
            Ok(view)
        },
    )
    .await?;

    Ok(Json(view))
}

/// Update a connection's label, config and/or secrets.
pub async fn update_connection(
    State(state): State<AppState>,
    Path((repo, account_id)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<UpsertConnectionRequest>,
) -> Result<Json<ConnectionView>, ApiError> {
    require_admin(auth.as_deref())?;
    let secret_box = SecretBox::new(&state.get_master_key().map_err(|_| {
        ApiError::internal("RAISIN_MASTER_KEY is not configured; cannot store connection secrets")
    })?);

    let config = reject_secrets_in_config(req.config.clone(), &req.secrets)?;

    let view = with_accounts_lock(
        &state,
        &tenant.tenant_id,
        &repo,
        &req.integration_path,
        "integration-connections",
        move |node| {
            let mut accounts = parse_accounts(node);
            let account = accounts
                .iter_mut()
                .find(|a| a.id == account_id)
                .ok_or_else(|| {
                    ApiError::new(
                        axum::http::StatusCode::NOT_FOUND,
                        "NOT_FOUND",
                        "no such connection on this connector",
                    )
                })?;

            if let Some(label) = req.label.clone() {
                account.label = Some(label).filter(|s| !s.is_empty());
            }
            if req.config.is_some() {
                account.config = Some(config.clone());
            }
            if !req.secrets.is_empty() {
                // Merge onto the existing map so an edit form can submit only
                // the fields the operator actually retyped.
                let existing = account
                    .secrets_encrypted
                    .as_deref()
                    .and_then(|enc| secret_box.decrypt_json(enc).ok())
                    .and_then(|v| match v {
                        Value::Object(m) => Some(m),
                        _ => None,
                    })
                    .unwrap_or_default();
                let secrets = encrypt_secrets(&existing, &req.secrets, &secret_box)?;
                account.secrets_encrypted = secrets.blob;
                account.secret_fields = secrets.field_names;
            }

            let view = ConnectionView::from(&*account);
            write_accounts(node, accounts)?;
            Ok(view)
        },
    )
    .await?;

    Ok(Json(view))
}

#[derive(Debug, Serialize)]
pub struct DeleteConnectionResponse {
    pub deleted: bool,
}

/// Remove a connection.
///
/// Distinct from `oauth/disconnect`, which additionally attempts a provider-side
/// token revoke. This is the plain removal used by credential-based connections
/// (there is nothing to revoke) and is safe for OAuth ones too.
pub async fn delete_connection(
    State(state): State<AppState>,
    Path((repo, account_id)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<DeleteConnectionResponse>, ApiError> {
    require_admin(auth.as_deref())?;

    let deleted = with_accounts_lock(
        &state,
        &tenant.tenant_id,
        &repo,
        &q.integration_path,
        "integration-connections",
        move |node| {
            let mut accounts = parse_accounts(node);
            let before = accounts.len();
            accounts.retain(|a| a.id != account_id);
            let removed = accounts.len() != before;
            if removed {
                write_accounts(node, accounts)?;
            }
            Ok(removed)
        },
    )
    .await?;

    Ok(Json(DeleteConnectionResponse { deleted }))
}

// --- helpers ---------------------------------------------------------------

/// Parse `connected_accounts` into typed connections, dropping unparseable
/// entries rather than failing the whole request.
fn parse_accounts(node: &raisin_models::nodes::Node) -> Vec<ConnectedAccount> {
    super::connected_accounts(node)
        .into_iter()
        .filter_map(|v| serde_json::from_value::<ConnectedAccount>(v).ok())
        .collect()
}

/// Serialize connections back onto the node.
fn write_accounts(
    node: &mut raisin_models::nodes::Node,
    accounts: Vec<ConnectedAccount>,
) -> Result<(), ApiError> {
    let values = accounts
        .into_iter()
        .map(|a| serde_json::to_value(a).unwrap_or(Value::Null))
        .collect::<Vec<_>>();
    super::set_connected_accounts(node, values)
}

struct EncryptedSecrets {
    blob: Option<String>,
    field_names: Vec<String>,
}

/// Merge `updates` onto `existing` and re-encrypt. A `None`/empty value clears
/// that field; an absent field is untouched.
fn encrypt_secrets(
    existing: &Map<String, Value>,
    updates: &BTreeMap<String, Option<String>>,
    secret_box: &SecretBox,
) -> Result<EncryptedSecrets, ApiError> {
    let mut map = existing.clone();
    for (field, value) in updates {
        match value {
            Some(v) if !v.is_empty() => {
                map.insert(field.clone(), Value::String(v.clone()));
            }
            _ => {
                map.remove(field);
            }
        }
    }
    if map.is_empty() {
        return Ok(EncryptedSecrets {
            blob: None,
            field_names: Vec::new(),
        });
    }
    let mut field_names: Vec<String> = map.keys().cloned().collect();
    field_names.sort();
    let blob = secret_box
        .encrypt_json(&Value::Object(map))
        .map_err(|e| ApiError::internal(format!("failed to encrypt connection secrets: {e}")))?;
    Ok(EncryptedSecrets {
        blob: Some(blob),
        field_names,
    })
}

/// Reject a secret arriving in the plaintext `config` object.
///
/// Secrets must travel in `secrets` so they are encrypted; a value that slipped
/// into `config` would be stored in cleartext on the node and returned by every
/// subsequent read. Failing the request is the only safe response — silently
/// moving it would leave the caller believing a different field was set.
fn reject_secrets_in_config(
    config: Option<Value>,
    secrets: &BTreeMap<String, Option<String>>,
) -> Result<Value, ApiError> {
    let config = config.unwrap_or(Value::Object(Map::new()));
    if let Value::Object(map) = &config {
        for key in map.keys() {
            if secrets.contains_key(key) {
                return Err(ApiError::validation_failed(format!(
                    "`{key}` is a secret field and must be sent under `secrets`, not `config` — \
                     values in `config` are stored in cleartext"
                )));
            }
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(k: &str) -> BTreeMap<String, Option<String>> {
        BTreeMap::from([(k.to_string(), Some("value".to_string()))])
    }

    /// A secret sent in `config` would be persisted in cleartext on the node and
    /// returned by every subsequent read, so it must be refused outright rather
    /// than quietly relocated.
    #[test]
    fn a_secret_sent_in_config_is_rejected() {
        let err = reject_secrets_in_config(
            Some(json!({ "host": "imap.example.com", "password": "oops" })),
            &key("password"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("password"));
    }

    #[test]
    fn non_secret_config_passes_through() {
        let cfg = reject_secrets_in_config(
            Some(json!({ "host": "imap.example.com" })),
            &key("password"),
        )
        .expect("host is not a secret");
        assert_eq!(cfg["host"], "imap.example.com");
    }

    #[test]
    fn absent_config_becomes_an_empty_object() {
        let cfg = reject_secrets_in_config(None, &BTreeMap::new()).unwrap();
        assert_eq!(cfg, json!({}));
    }

    fn box_for_tests() -> SecretBox {
        SecretBox::new(&[7u8; 32])
    }

    #[test]
    fn secrets_merge_so_an_edit_need_not_resend_untouched_fields() {
        let sb = box_for_tests();
        let existing = Map::from_iter([
            ("password".to_string(), json!("old-pw")),
            ("api_key".to_string(), json!("keep-me")),
        ]);
        let updates = BTreeMap::from([("password".to_string(), Some("new-pw".to_string()))]);

        let out = encrypt_secrets(&existing, &updates, &sb).unwrap();
        let decrypted = sb.decrypt_json(&out.blob.unwrap()).unwrap();

        assert_eq!(decrypted["password"], "new-pw");
        // Untouched field survives — the edit form never saw it.
        assert_eq!(decrypted["api_key"], "keep-me");
        assert_eq!(out.field_names, vec!["api_key", "password"]);
    }

    #[test]
    fn a_null_value_clears_one_field() {
        let sb = box_for_tests();
        let existing = Map::from_iter([
            ("password".to_string(), json!("pw")),
            ("api_key".to_string(), json!("k")),
        ]);
        let updates = BTreeMap::from([("password".to_string(), None)]);

        let out = encrypt_secrets(&existing, &updates, &sb).unwrap();
        let decrypted = sb.decrypt_json(&out.blob.unwrap()).unwrap();
        assert!(decrypted.get("password").is_none());
        assert_eq!(out.field_names, vec!["api_key"]);
    }

    #[test]
    fn clearing_the_last_secret_drops_the_blob_entirely() {
        let sb = box_for_tests();
        let existing = Map::from_iter([("password".to_string(), json!("pw"))]);
        let updates = BTreeMap::from([("password".to_string(), None)]);

        let out = encrypt_secrets(&existing, &updates, &sb).unwrap();
        assert!(out.blob.is_none());
        assert!(out.field_names.is_empty());
    }

    /// The projection is the reason the console reads connections from here
    /// rather than off the raw node: no ciphertext reaches the browser.
    #[test]
    fn the_connection_view_never_carries_secret_material() {
        let account = ConnectedAccount {
            id: "c1".into(),
            label: Some("Support".into()),
            tokens_encrypted: Some("TOKEN-CIPHERTEXT".into()),
            secrets_encrypted: Some("SECRET-CIPHERTEXT".into()),
            secret_fields: vec!["password".into()],
            config: Some(json!({ "host": "imap.example.com" })),
            ..Default::default()
        };

        let rendered = serde_json::to_string(&ConnectionView::from(&account)).unwrap();
        assert!(!rendered.contains("TOKEN-CIPHERTEXT"));
        assert!(!rendered.contains("SECRET-CIPHERTEXT"));
        // But the operator can still see WHICH secrets are set, and the config.
        assert!(rendered.contains("password"));
        assert!(rendered.contains("imap.example.com"));
        assert!(rendered.contains("Support"));
    }
}
