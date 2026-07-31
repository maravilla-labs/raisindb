// SPDX-License-Identifier: BSL-1.1

//! `POST /api/integrations/{repo}/test` — the "Test connection" / connector
//! debug endpoint.
//!
//! Runs a synchronous, bounded diagnostic against a connector's adapter: it
//! invokes the adapter's `capabilities` operation, then a small `list` probe,
//! and returns a structured report the admin UI renders. On success it caches
//! the resolved capabilities onto the `raisin:Integration` node so the UI can
//! read them without re-testing.
//!
//! Security invariants: no `access_token`, `refresh_token`, or `client_secret`
//! ever appears in a response body or log line. The credential handed to the
//! adapter has its `refresh_token` stripped (see [`support::build_credential`]),
//! and the probe `sample` carries item **names** only — never URLs, which may
//! embed tokens.

mod probe;
mod support;

use std::time::{Duration, Instant};

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use serde::{Deserialize, Serialize};

use super::require_admin;
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Branch adapter functions are resolved on.
const FUNCTIONS_BRANCH: &str = "main";
/// Workspace adapter functions live in.
const FUNCTIONS_WORKSPACE: &str = "functions";
/// Wall-clock ceiling for the whole probe so a hung adapter cannot hang the
/// request.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum probe items surfaced (names only).
const PROBE_LIMIT: usize = 10;

/// Request body for the connection test.
#[derive(Debug, Deserialize)]
pub struct TestConnectionRequest {
    /// Path of the `raisin:Integration` node to test.
    pub integration_path: String,
    /// Connected-account id to test with. When omitted the adapter is invoked
    /// with `credential: null` and `auth` is reported as `not_required`.
    #[serde(default)]
    pub account_id: Option<String>,
    /// Remote folder to probe. Defaults to the adapter's own root.
    #[serde(default)]
    pub remote_root: Option<String>,
    /// Caller-supplied `sync_config` keys to forward into the probe's mount
    /// snapshot (e.g. ms-graph's `resource`, a calendar `window`).
    ///
    /// Without this the probe always synthesized a bare `sync_config`, so
    /// `resourceOf(mount)` fell back to `"mail"` and a calendar or OneDrive
    /// mount could never be tested against the surface it actually syncs.
    /// Engine-owned keys below still win, so a caller cannot widen the probe.
    #[serde(default)]
    pub sync_config: Option<serde_json::Value>,
}

/// The `capabilities` object cached on the integration node and echoed back to
/// the UI. Field-for-field mirror of the sync engine's `Capabilities`
/// (`raisin-rocksdb` … `virtual_mount_sync::adapter`); duplicated here because
/// that crate is an optional dependency and does not re-export the type at its
/// crate root. Every field defaults conservatively.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub can_read: bool,
    #[serde(default)]
    pub can_write: bool,
    #[serde(default)]
    pub can_create_folders: bool,
    #[serde(default)]
    pub supports_changes: bool,
    #[serde(default)]
    pub supports_webhooks: bool,
    #[serde(default)]
    pub supports_search: bool,
    #[serde(default)]
    pub supports_push: bool,
    #[serde(default)]
    pub default_ttl: Option<u64>,
    #[serde(default)]
    pub max_file_size: Option<i64>,
}

/// The bounded `list` probe result. Names only — never URLs.
#[derive(Debug, Serialize)]
pub struct Probe {
    pub items_seen: usize,
    pub sample: Vec<String>,
}

/// A structured diagnostic error.
#[derive(Debug, Serialize)]
pub struct TestError {
    pub code: String,
    pub message: String,
}

/// Response for the connection test. A failed connection is still a `200` — it
/// is a diagnostic result, not a server error.
#[derive(Debug, Serialize)]
pub struct TestConnectionResponse {
    pub ok: bool,
    pub latency_ms: u64,
    /// `"valid" | "expired" | "missing" | "not_required"`.
    pub auth: String,
    pub capabilities: Option<Capabilities>,
    pub probe: Option<Probe>,
    pub error: Option<TestError>,
}

/// Internal probe result, folded into the response.
pub(super) struct ProbeOutcome {
    pub ok: bool,
    pub auth: String,
    pub capabilities: Option<Capabilities>,
    pub probe: Option<Probe>,
    pub error: Option<TestError>,
}

impl ProbeOutcome {
    fn timed_out(base_auth: &str) -> Self {
        Self {
            ok: false,
            auth: base_auth.to_string(),
            capabilities: None,
            probe: None,
            error: Some(TestError {
                code: "timeout".to_string(),
                message: format!(
                    "adapter did not respond within {}s",
                    PROBE_TIMEOUT.as_secs()
                ),
            }),
        }
    }
}

/// Test / debug a connector's adapter.
#[cfg(feature = "storage-rocksdb")]
pub async fn test_connection(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<TestConnectionRequest>,
) -> Result<Json<TestConnectionResponse>, ApiError> {
    require_admin(auth.as_deref())?;
    let started = Instant::now();

    let svc = super::config_service(&state, &tenant.tenant_id, &repo, "integration-test");
    let mut node = svc
        .get_by_path(&req.integration_path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(req.integration_path.clone()))?;

    let provider_type = support::string_prop(&node, "provider_type").unwrap_or_default();
    let adapter_path = support::string_prop(&node, "adapter_function")
        .ok_or_else(|| ApiError::validation_failed("integration is missing an adapter_function"))?;

    // Resolve the credential (refresh_token stripped) and the base auth state.
    let (credential, base_auth) = match &req.account_id {
        // No account was named. "not_required" is only true for adapters that
        // genuinely need no credential; for anything OAuth-shaped, invoking with
        // `credential: null` just pushes the failure into the adapter, where it
        // used to surface as an opaque JS TypeError. Diagnose it here instead.
        None if requires_credential(&node) => {
            return Ok(Json(fail(
                started,
                "missing",
                "missing_credential",
                "this connector authenticates per account — connect an account and \
                 select it before testing",
            )));
        }
        None => (None, "not_required"),
        Some(id) => match support::resolve_credential(&state, &node, &provider_type, id) {
            Some(cred) => (Some(cred), "valid"),
            None => {
                return Ok(Json(fail(
                    started,
                    "missing",
                    "missing_credential",
                    "no decryptable token for the requested account",
                )));
            }
        },
    };

    // Resolve the adapter function node once.
    let adapter_node =
        match probe::load_adapter_node(&state, &tenant.tenant_id, &repo, &adapter_path).await? {
            Some(n) => n,
            None => {
                return Ok(Json(fail(
                    started,
                    base_auth,
                    "adapter_not_found",
                    &format!("adapter function `{adapter_path}` not found"),
                )));
            }
        };

    let api_config = node
        .properties
        .get("api_config")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(serde_json::Value::Null);
    let mount = support::mount_snapshot(&req.remote_root, &api_config, req.sync_config.as_ref());
    let outcome = match tokio::time::timeout(
        PROBE_TIMEOUT,
        probe::run_probe(
            &state,
            &tenant.tenant_id,
            &repo,
            &adapter_node,
            &credential,
            &mount,
            &req.remote_root,
            base_auth,
        ),
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => ProbeOutcome::timed_out(base_auth),
    };

    // Persist resolved capabilities on success so the UI need not re-test.
    if outcome.ok {
        if let Some(caps) = &outcome.capabilities {
            let _ = support::cache_capabilities(&svc, &mut node, caps).await;
        }
    }

    Ok(Json(TestConnectionResponse {
        ok: outcome.ok,
        latency_ms: started.elapsed().as_millis() as u64,
        auth: outcome.auth,
        capabilities: outcome.capabilities,
        probe: outcome.probe,
        error: outcome.error,
    }))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn test_connection(
    State(_state): State<AppState>,
    Path(_repo): Path<String>,
    Extension(_tenant): Extension<TenantInfo>,
    _auth: Option<Extension<AuthContext>>,
    Json(_req): Json<TestConnectionRequest>,
) -> Result<Json<TestConnectionResponse>, ApiError> {
    Err(ApiError::internal(
        "Connection test requires the RocksDB backend",
    ))
}

/// Whether this connector authenticates per connected account.
///
/// True when it declares an OAuth authorization endpoint or already has at
/// least one connected account. Both signals matter: a freshly configured OAuth
/// connector has no accounts yet, and a credential-based connector may have
/// accounts without an `oauth_config`. Adapters that need nothing (a public
/// API) declare neither and keep the `not_required` path.
#[cfg(feature = "storage-rocksdb")]
fn requires_credential(node: &raisin_models::nodes::Node) -> bool {
    let has_auth_url = super::json_str(&super::oauth_config(node), "auth_url").is_some();
    has_auth_url || !super::connected_accounts(node).is_empty()
}

/// Build a failed diagnostic response (still HTTP 200).
#[cfg(feature = "storage-rocksdb")]
fn fail(started: Instant, auth: &str, code: &str, message: &str) -> TestConnectionResponse {
    TestConnectionResponse {
        ok: false,
        latency_ms: started.elapsed().as_millis() as u64,
        auth: auth.to_string(),
        capabilities: None,
        probe: None,
        error: Some(TestError {
            code: code.to_string(),
            message: message.to_string(),
        }),
    }
}
