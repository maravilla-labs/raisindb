// SPDX-License-Identifier: BSL-1.1

//! Operator package-provisioning routes.
//!
//! Mounted by raisin-server under the per-tenant `/management/*` subtree,
//! which is layered with `ensure_tenant_middleware` +
//! `require_admin_auth_middleware`. That middleware accepts per-tenant admin
//! JWTs and — when `RAISIN_SUPERADMIN_TOKEN` is configured — the operator
//! superadmin bearer (synthesized into tenant-scoped admin claims), so a
//! hosting control plane can provision packages into a tenant without any
//! change to the customer-facing `/api` auth surface.
//!
//! Self-hosted deployments are unaffected: without the operator token these
//! routes still require a valid per-tenant admin JWT, exactly like the rest
//! of `/management/*`.

use axum::{
    body::Body,
    extract::DefaultBodyLimit,
    http::Request,
    middleware::{from_fn, Next},
    response::Response,
    routing::{any, post},
    Router,
};

use crate::state::AppState;

use super::MAX_UPLOAD_SIZE;

/// Bridge `require_admin_auth_middleware` (which only inserts `AdminClaims`)
/// to the package handlers, which read an `AuthContext` extension for RLS.
/// An authenticated admin without impersonation operates as system context —
/// the same privilege `require_auth_middleware` resolves for admin
/// principals. Without claims, no context is inserted and RLS denies.
#[cfg(feature = "storage-rocksdb")]
async fn admin_system_auth_context(mut req: Request<Body>, next: Next) -> Response {
    use raisin_models::auth::AuthContext;
    if req.extensions().get::<AuthContext>().is_none()
        && req.extensions().get::<raisin_rocksdb::AdminClaims>().is_some()
    {
        req.extensions_mut().insert(AuthContext::system());
    }
    next.run(req).await
}

/// Build the operator package routes (upload + unified package command).
///
/// - `POST /management/packages/{repo}/upload` — multipart `.rap` upload into
///   the tenant's `packages` workspace (same handler as the customer upload).
/// - `GET/POST /management/packages/{repo}/{branch}/head/{*path}` — unified
///   package command endpoint (`raisin:install?mode=sync`, `raisin:browse`,
///   `raisin:file`), mirroring `/api/packages/...`.
#[cfg(feature = "storage-rocksdb")]
pub fn operator_package_routes(state: AppState) -> Router {
    #[allow(deprecated)]
    Router::new()
        .route(
            "/management/packages/{repo}/upload",
            post(crate::handlers::packages::upload_package)
                .layer(DefaultBodyLimit::max(MAX_UPLOAD_SIZE)),
        )
        .route(
            "/management/packages/{repo}/{branch}/head/{*path}",
            any(crate::handlers::packages::handle_package_command),
        )
        .layer(from_fn(admin_system_auth_context))
        .with_state(state)
}
