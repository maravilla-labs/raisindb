// SPDX-License-Identifier: BSL-1.1

//! Routes for authentication, identity management, workspace access control,
//! admin user management, identity user management, and user profile/API keys.
//!
//! All routes in this module require the `storage-rocksdb` feature.

use axum::Router;

use crate::state::AppState;

/// Build authentication and identity routes (RocksDB only).
///
/// Includes: system auth, CLI auth, identity auth (local, magic-link, OIDC),
/// repository-scoped auth, workspace access control, admin users,
/// identity users, and user profile/API key management.
#[cfg(feature = "storage-rocksdb")]
pub(crate) fn auth_routes(state: &AppState) -> Router<AppState> {
    use crate::middleware::require_auth_middleware;
    use axum::middleware::from_fn_with_state;
    use axum::routing::{get, post};

    Router::new()
        // ----------------------------------------------------------------
        // System authentication (tenant-scoped)
        // ----------------------------------------------------------------
        .route(
            "/api/raisindb/sys/{tenant_id}/auth",
            post(crate::handlers::auth::authenticate),
        )
        // CLI authentication (browser-based login flow)
        .route("/auth/cli", get(crate::handlers::auth::cli_auth_page))
        .route(
            "/auth/cli/login",
            post(crate::handlers::auth::cli_auth_login),
        )
        // Password change (requires auth)
        .route(
            "/api/raisindb/sys/{tenant_id}/auth/change-password",
            post(crate::handlers::auth::change_password)
                .layer(from_fn_with_state(state.clone(), require_auth_middleware)),
        )
        // ----------------------------------------------------------------
        // Identity Authentication (pluggable auth system)
        // ----------------------------------------------------------------
        .route(
            "/auth/providers",
            get(crate::handlers::identity_auth::get_providers),
        )
        .route(
            "/auth/register",
            post(crate::handlers::identity_auth::register),
        )
        .route("/auth/login", post(crate::handlers::identity_auth::login))
        // Magic link (passwordless)
        .route(
            "/auth/magic-link",
            post(crate::handlers::identity_auth::request_magic_link),
        )
        .route(
            "/auth/magic-link/verify",
            get(crate::handlers::identity_auth::verify_magic_link),
        )
        // OIDC (Google, Okta, Keycloak, Azure AD)
        .route(
            "/auth/oidc/{provider}",
            get(crate::handlers::identity_auth::oidc_authorize),
        )
        .route(
            "/auth/oidc/{provider}/callback",
            get(crate::handlers::identity_auth::oidc_callback),
        )
        // Token refresh
        .route(
            "/auth/refresh",
            post(crate::handlers::identity_auth::refresh_token),
        )
        // Logout (requires auth)
        .route("/auth/logout", post(crate::handlers::identity_auth::logout))
        // Session management (requires auth)
        .route(
            "/auth/sessions",
            get(crate::handlers::identity_auth::list_sessions),
        )
        .route(
            "/auth/sessions/{session_id}",
            axum::routing::delete(crate::handlers::identity_auth::revoke_session),
        )
        // Current identity info (requires auth)
        .route("/auth/me", get(crate::handlers::identity_auth::get_me))
        // ----------------------------------------------------------------
        // Repository-Scoped Authentication
        // ----------------------------------------------------------------
        .route(
            "/auth/{repo}/register",
            post(crate::handlers::identity_auth::register_for_repo),
        )
        .route(
            "/auth/{repo}/login",
            post(crate::handlers::identity_auth::login_for_repo),
        )
        .route(
            "/auth/{repo}/refresh",
            post(crate::handlers::identity_auth::refresh_token),
        )
        .route(
            "/auth/{repo}/magic-link",
            post(crate::handlers::identity_auth::request_magic_link_for_repo),
        )
        .route(
            "/auth/{repo}/providers",
            get(crate::handlers::identity_auth::get_providers_for_repo),
        )
        .route(
            "/auth/{repo}/me",
            get(crate::handlers::identity_auth::get_me_for_repo),
        )
        // ----------------------------------------------------------------
        // Workspace Access Control
        // ----------------------------------------------------------------
        .route(
            "/repos/{repo}/access/request",
            post(crate::handlers::workspace_access::request_access),
        )
        .route(
            "/repos/{repo}/access/requests",
            get(crate::handlers::workspace_access::list_requests),
        )
        .route(
            "/repos/{repo}/access/approve/{request_id}",
            post(crate::handlers::workspace_access::approve_request),
        )
        .route(
            "/repos/{repo}/access/deny/{request_id}",
            post(crate::handlers::workspace_access::deny_request),
        )
        .route(
            "/repos/{repo}/access/invite",
            post(crate::handlers::workspace_access::invite_user),
        )
        .route(
            "/repos/{repo}/access/revoke/{identity_id}",
            post(crate::handlers::workspace_access::revoke_access),
        )
        .route(
            "/repos/{repo}/access/members",
            get(crate::handlers::workspace_access::list_members),
        )
        // ----------------------------------------------------------------
        // Admin users management (requires auth)
        // ----------------------------------------------------------------
        .route(
            "/api/raisindb/sys/{tenant_id}/admin-users",
            get(crate::handlers::admin_users::list_admin_users)
                .post(crate::handlers::admin_users::create_admin_user)
                .layer(from_fn_with_state(state.clone(), require_auth_middleware)),
        )
        .route(
            "/api/raisindb/sys/{tenant_id}/admin-users/{username}",
            get(crate::handlers::admin_users::get_admin_user)
                .put(crate::handlers::admin_users::update_admin_user)
                .delete(crate::handlers::admin_users::delete_admin_user)
                .layer(from_fn_with_state(state.clone(), require_auth_middleware)),
        )
        // ----------------------------------------------------------------
        // Identity users management (requires auth)
        // ----------------------------------------------------------------
        .route(
            "/api/raisindb/sys/{tenant_id}/identity-users",
            get(crate::handlers::identity_users::list_identity_users)
                .post(crate::handlers::identity_users::create_identity_user)
                .layer(from_fn_with_state(state.clone(), require_auth_middleware)),
        )
        .route(
            "/api/raisindb/sys/{tenant_id}/identity-users/{identity_id}",
            get(crate::handlers::identity_users::get_identity_user)
                .patch(crate::handlers::identity_users::update_identity_user)
                .delete(crate::handlers::identity_users::delete_identity_user)
                .layer(from_fn_with_state(state.clone(), require_auth_middleware)),
        )
        .route(
            "/api/raisindb/sys/{tenant_id}/identity-users/{identity_id}/link",
            post(crate::handlers::identity_users::link_identity_user)
                .layer(from_fn_with_state(state.clone(), require_auth_middleware)),
        )
        // ----------------------------------------------------------------
        // User profile and API key management (requires auth)
        // ----------------------------------------------------------------
        .route(
            "/api/raisindb/me",
            get(crate::handlers::profile::get_profile)
                .layer(from_fn_with_state(state.clone(), require_auth_middleware)),
        )
        .route(
            "/api/raisindb/me/api-keys",
            get(crate::handlers::profile::list_api_keys)
                .post(crate::handlers::profile::create_api_key)
                .layer(from_fn_with_state(state.clone(), require_auth_middleware)),
        )
        .route(
            "/api/raisindb/me/api-keys/{key_id}",
            axum::routing::delete(crate::handlers::profile::revoke_api_key)
                .layer(from_fn_with_state(state.clone(), require_auth_middleware)),
        )
        .route(
            "/api/raisindb/me/repositories",
            get(crate::handlers::profile::list_repositories)
                .layer(from_fn_with_state(state.clone(), require_auth_middleware)),
        )
        // ----------------------------------------------------------------
        // Tenant authentication configuration
        // ----------------------------------------------------------------
        .route(
            "/api/tenants/{tenant_id}/auth/config",
            get(crate::handlers::identity_auth::get_auth_config)
                .put(crate::handlers::identity_auth::update_auth_config),
        )
}
