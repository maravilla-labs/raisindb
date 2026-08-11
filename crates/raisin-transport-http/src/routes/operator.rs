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
        && req
            .extensions()
            .get::<raisin_rocksdb::AdminClaims>()
            .is_some()
    {
        req.extensions_mut().insert(AuthContext::system());
    }
    next.run(req).await
}

/// Build the operator routes: packages, plus the integration credentials a control
/// plane must write.
///
/// - `POST /management/packages/{repo}/upload` — multipart `.rap` upload into
///   the tenant's `packages` workspace (same handler as the customer upload).
/// - `GET/POST /management/packages/{repo}/{branch}/head/{*path}` — unified
///   package command endpoint (`raisin:install?mode=sync`, `raisin:browse`,
///   `raisin:file`), mirroring `/api/packages/...`.
/// - `POST /management/integrations/{repo}/oauth/client-secret` — the operator
///   counterpart of `/api/integrations/{repo}/oauth/client-secret`, for connectors
///   whose OAuth client belongs to the platform rather than to the customer.
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
        // Same handler as `/api/integrations/{repo}/oauth/client-secret`, reachable by the
        // OPERATOR rather than by a tenant admin.
        //
        // A managed connector — one whose OAuth client belongs to the platform rather than
        // to the customer — cannot be completed from the console by design: it declares an
        // empty OAuth surface precisely so an operator is never shown credentials they do
        // not own. The control plane has to write them instead, and it authenticates with
        // the superadmin token, which `require_auth_middleware` does not accept. So without
        // this route no principal could provision such a connector at all: the console has
        // no field, and the control plane got a 401.
        //
        // This adds a door, not a key. The handler is unchanged and still calls
        // `require_admin`; `admin_system_auth_context` below supplies the system
        // `AuthContext` that satisfies it, exactly as it already does for package installs.
        // `/api/integrations/*` is untouched.
        .route(
            "/management/integrations/{repo}/oauth/client-secret",
            post(crate::handlers::integrations::set_client_secret),
        )
        .layer(from_fn(admin_system_auth_context))
        .with_state(state)
}
