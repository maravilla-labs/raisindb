// SPDX-License-Identifier: BSL-1.1

//! Admin SPA bootstrap endpoint.
//!
//! `GET /api/admin/bootstrap` returns the per-request tenant identifier, the
//! running server version, a `dev_mode` flag, and — only in dev mode — the
//! superadmin token so the SPA can transparently call `/management/admin/*`
//! during local development.
//!
//! The admin SPA fetches this on boot so it can render a tenant badge, the
//! server version, and gate dev-only UI without hard-coding a tenant
//! identifier in the bundle.
//!
//! Security notes
//! --------------
//! The `superadmin_token` field is exposed **only when `dev_mode == true`**.
//! In production builds (where `dev_mode == false`) the field is omitted from
//! the JSON payload entirely (via `serde(skip_serializing_if)`), regardless
//! of whether `RAISIN_SUPERADMIN_TOKEN` is set. Each time the token is
//! actually surfaced we emit a `tracing::warn!` so it's visible in logs.
//!
//! The response is per-request (depends on the `x-tenant-id` header that the
//! tenant middleware resolves) so we set `Cache-Control: no-store`. The rest
//! of the admin SPA assets (HTML, JS, CSS) remain aggressively cacheable.

use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue},
    response::IntoResponse,
    Extension, Json,
};
use serde::Serialize;

use crate::middleware::TenantInfo;
use crate::state::AppState;

/// Response body for the bootstrap endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct BootstrapResponse {
    /// Tenant identifier resolved by `ensure_tenant_middleware` from the
    /// `x-tenant-id` request header (defaults to `"default"` when absent).
    pub tenant_id: String,
    /// Server version (from `CARGO_PKG_VERSION` of the raisin-server binary).
    pub version: String,
    /// Whether the server is running in dev/single-operator mode.
    pub dev_mode: bool,
    /// Superadmin token, exposed **only in dev mode** as a convenience for the
    /// admin SPA (so it can attach `X-Raisin-Superadmin-Token` automatically
    /// when calling `/management/admin/*`). In production this field is
    /// omitted from the JSON entirely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superadmin_token: Option<String>,
}

/// `GET /api/admin/bootstrap`
///
/// Returns `{ tenant_id, version, dev_mode }` plus an optional
/// `superadmin_token` (dev-mode only). The handler is intentionally auth-free:
/// it only echoes server-side facts that depend on the request tenant header
/// (already resolved by `ensure_tenant_middleware`).
pub async fn bootstrap(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
) -> impl IntoResponse {
    // The token is resolved once at startup and cached on AppState only when
    // dev_mode is on (see `router_with_bin_and_audit`). In production the
    // field is `None` regardless of env state, so this handler structurally
    // cannot leak it.
    let superadmin_token = state.superadmin_token.as_ref().map(|t| {
        tracing::warn!(
            tenant_id = %tenant_info.tenant_id,
            "Bootstrap endpoint serving superadmin token to client (dev_mode=true)"
        );
        t.to_string()
    });

    let body = BootstrapResponse {
        tenant_id: tenant_info.tenant_id,
        version: state.server_version.clone(),
        dev_mode: state.dev_mode,
        superadmin_token,
    };

    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));

    (headers, Json(body))
}
