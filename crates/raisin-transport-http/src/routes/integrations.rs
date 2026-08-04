// SPDX-License-Identifier: BSL-1.1

//! Routes for outbound integrations (connect an external system, refresh/rotate
//! its tokens, trigger a mount sync).
//!
//! These are namespaced `/api/integrations/...` and are entirely separate from
//! [`crate::routes::oauth`], which is the *inbound* OAuth 2.1 authorization
//! server for MCP clients.
//!
//! All mutating endpoints run behind `require_auth_middleware` and are further
//! admin-gated inside each handler. The one exception is the OAuth **callback**:
//! it is a browser redirect from the provider and therefore cannot carry a
//! bearer token, so it is authenticated by the single-use, TTL'd `state`
//! parameter that the authenticated `oauth/start` call minted.

use axum::Router;

use crate::state::AppState;

/// Build the outbound-integrations route group.
///
/// Empty when the `storage-rocksdb` feature is disabled (integration config and
/// job enqueue both require the RocksDB backend).
pub(crate) fn integration_routes(state: &AppState) -> Router<AppState> {
    #[cfg(feature = "storage-rocksdb")]
    {
        use crate::handlers::integrations;
        use crate::middleware::require_auth_middleware;
        use axum::middleware::from_fn_with_state;
        use axum::routing::{get, post};

        // Public: providers cannot send a bearer token.
        // - OAuth callback: authenticated by the single-use `state` parameter.
        // - notifications: provider-facing push endpoint, guarded by the
        //   unguessable per-mount token in the path plus the stored per-mount
        //   secret. Both GET (validation handshake / health) and POST
        //   (invalidation signal) are accepted.
        let public = Router::new()
            .route(
                "/api/integrations/{repo}/oauth/callback",
                get(integrations::callback),
            )
            .route(
                "/api/integrations/{repo}/notifications/{mount_token}",
                get(integrations::notify).post(integrations::notify),
            );

        // Admin-gated: require a valid principal AND admin authorization.
        let guarded = Router::new()
            .route(
                "/api/integrations/{repo}/oauth/start",
                post(integrations::start),
            )
            .route(
                "/api/integrations/{repo}/oauth/disconnect",
                post(integrations::disconnect),
            )
            .route(
                "/api/integrations/{repo}/oauth/client-secret",
                post(integrations::set_client_secret),
            )
            // Generic connector-level secrets (supersedes oauth/client-secret,
            // which stays registered as the narrow legacy alias).
            .route(
                "/api/integrations/{repo}/config-secrets",
                axum::routing::put(integrations::set_config_secrets),
            )
            // Connections: the non-OAuth path to a connected account, and what
            // lets one connector hold several.
            .route(
                "/api/integrations/{repo}/connections",
                get(integrations::list_connections).post(integrations::create_connection),
            )
            .route(
                "/api/integrations/{repo}/connections/{account_id}",
                axum::routing::patch(integrations::update_connection)
                    .delete(integrations::delete_connection),
            )
            .route(
                "/api/integrations/{repo}/mounts/{mount_id}/sync",
                post(integrations::sync_mount),
            )
            // Pause suspends scheduling and KEEPS the provider subscription;
            // stop ends the running pass and discards its resume point. Both
            // write only the fields they own — deliberately not the generic node
            // API, which replaces every property.
            .route(
                "/api/integrations/{repo}/mounts/{mount_id}/pause",
                post(integrations::pause_mount),
            )
            .route(
                "/api/integrations/{repo}/mounts/{mount_id}/stop",
                post(integrations::stop_mount),
            )
            // Live sync progress. SSE rather than WS deliberately: `fetch`
            // carries the ordinary bearer token, which a WS handshake cannot.
            .route(
                "/api/integrations/{repo}/mounts/{mount_id}/events",
                get(integrations::stream_mount_events),
            )
            // Deleting a mount MUST go through here, not a generic node delete:
            // this is the only path that unregisters the provider's webhook
            // subscription first. Without it every deleted mount leaks a live
            // subscription and the provider keeps POSTing notifications at a URL
            // that 404s, indefinitely.
            .route(
                "/api/integrations/{repo}/mounts/{mount_id}",
                axum::routing::delete(integrations::delete_mount),
            )
            // Provider-side URLs the admin UI displays for the user to register.
            // Integration-level (redirect URI only, no mount required) + per-mount
            // (redirect URI + push notification URL; lazily mints the mount token).
            .route(
                "/api/integrations/{repo}/setup-urls",
                get(integrations::setup_urls_integration),
            )
            .route(
                "/api/integrations/{repo}/mounts/{mount_id}/setup-urls",
                get(integrations::setup_urls_mount),
            )
            .route(
                "/api/integrations/{repo}/test",
                post(integrations::test_connection),
            )
            // Remote discovery for the mount editor: list the provider's mail
            // folders / calendars / sites / drives so an operator picks one
            // instead of pasting an opaque id. Read-only, synchronous, bounded.
            .route(
                "/api/integrations/{repo}/browse",
                post(integrations::browse),
            )
            .layer(from_fn_with_state(state.clone(), require_auth_middleware));

        public.merge(guarded)
    }

    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let _ = state;
        Router::new()
    }
}
