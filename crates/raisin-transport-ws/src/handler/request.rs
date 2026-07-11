// SPDX-License-Identifier: BSL-1.1

//! Request processing and routing for WebSocket messages.

use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    protocol::{RequestEnvelope, ResponseEnvelope},
};

use super::state::WsState;

/// Process a single request
pub(super) async fn process_request<S, B>(
    state: Arc<WsState<S, B>>,
    connection_state: Arc<parking_lot::RwLock<ConnectionState>>,
    request: RequestEnvelope,
) where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let request_id = request.request_id.clone();
    tracing::info!(
        "process_request() started - request_id: {}, type: {:?}",
        request_id,
        request.request_type
    );

    // SECURITY: The tenant is owned by the connection — established at
    // upgrade time from the /sys path or x-tenant-id header, both of which
    // the edge controls — never by message content. Handlers read
    // `request.context.tenant_id`, so a client-supplied value would let any
    // socket read or write another tenant's data. Clamp it here, mirroring
    // the HTTP ensure_tenant semantics. This also fixes clients that bake
    // "default" into their message context (e.g. the Studio SPA), whose
    // queries previously landed in the wrong tenant.
    let mut request = request;
    {
        let conn = connection_state.read();
        if request.context.tenant_id != conn.tenant_id {
            tracing::debug!(
                request_tenant = %request.context.tenant_id,
                connection_tenant = %conn.tenant_id,
                request_id = %request_id,
                "Overriding request context tenant with connection tenant"
            );
            request.context.tenant_id = conn.tenant_id.clone();
        }
    }

    // SECURITY: The repository is connection-owned for the same reason as
    // the tenant. The connection's AuthContext (anonymous resolution or
    // identity permissions) is resolved once, scoped to the upgrade-time
    // repository — handlers pair the *message's* repository with that cached
    // context, so a client naming a different repo would have its RLS
    // evaluated against the wrong repo's permission set (e.g. reading a
    // repo with anonymous access disabled through a connection that was
    // anonymously authorized on another repo). Reject mismatches instead of
    // silently overriding so misbehaving clients surface. System contexts
    // (operator/admin) are exempt — cross-repo access within the clamped
    // tenant is their intended scope.
    let repo_scope_violation = {
        let conn = connection_state.read();
        let is_system = conn.auth_context().map(|a| a.is_system).unwrap_or(false);
        if is_system {
            None
        } else {
            // Connections without a repo in the upgrade path had their
            // permissions resolved for "default" — hold them to it.
            let conn_repo = conn
                .repository
                .clone()
                .unwrap_or_else(|| "default".to_string());
            match request.context.repository.as_deref() {
                Some(req_repo) if req_repo != conn_repo => Some((conn_repo, req_repo.to_string())),
                _ => None,
            }
        }
    };
    if let Some((conn_repo, req_repo)) = repo_scope_violation {
        tracing::warn!(
            connection_repo = %conn_repo,
            request_repo = %req_repo,
            request_id = %request_id,
            request_type = ?request.request_type,
            "Rejecting request addressing a repository outside the connection's auth scope"
        );
        let response = ResponseEnvelope::error(
            request_id.clone(),
            "REPOSITORY_SCOPE_MISMATCH".to_string(),
            format!(
                "This connection is scoped to repository '{}'; open a connection to '{}' to address it",
                conn_repo, req_repo
            ),
        );
        let conn = connection_state.read();
        let _ = conn.send_response(response);
        return;
    }

    // Check authentication if required
    let needs_auth = {
        let conn = connection_state.read();
        state.config.require_auth
            && !conn.is_authenticated()
            && request.request_type != crate::protocol::RequestType::Authenticate
            && request.request_type != crate::protocol::RequestType::AuthenticateJwt
    };

    if needs_auth {
        let response = ResponseEnvelope::error(
            request_id.clone(),
            "NOT_AUTHENTICATED".to_string(),
            "Authentication required".to_string(),
        );
        let conn = connection_state.read();
        let _ = conn.send_response(response);
        return;
    }

    // Acquire global semaphore permit if configured
    let _global_permit = if let Some(ref semaphore) = state.global_semaphore {
        match semaphore.try_acquire() {
            Ok(permit) => Some(permit),
            Err(_) => {
                let conn = connection_state.read();
                let response = ResponseEnvelope::error(
                    request_id,
                    "RATE_LIMIT_EXCEEDED".to_string(),
                    "Global rate limit exceeded".to_string(),
                );
                let _ = conn.send_response(response);
                return;
            }
        }
    } else {
        None
    };

    // Acquire per-connection semaphore permit
    let operation_semaphore = {
        let conn = connection_state.read();
        conn.get_operation_semaphore()
    };

    let _permit = match operation_semaphore.try_acquire() {
        Ok(permit) => permit,
        Err(_) => {
            let conn = connection_state.read();
            let response = ResponseEnvelope::error(
                request_id,
                "RATE_LIMIT_EXCEEDED".to_string(),
                "Too many concurrent operations".to_string(),
            );
            let _ = conn.send_response(response);
            return;
        }
    };

    // Route request to appropriate handler
    tracing::info!("Calling route_request for request_id: {}", request_id);
    let result = crate::handlers::route_request(&state, &connection_state, request).await;
    tracing::info!("route_request returned for request_id: {}", request_id);

    // Send response if not already sent (e.g., by streaming handler)
    match result {
        Ok(Some(response)) => {
            tracing::info!(
                "route_request returned Ok(Some(response)) for request_id: {}",
                request_id
            );
            let conn = connection_state.read();
            let _ = conn.send_response(response);
            tracing::info!("Response sending completed for request_id: {}", request_id);
        }
        Ok(None) => {
            tracing::info!(
                "route_request returned Ok(None) - response already sent for request_id: {}",
                request_id
            );
        }
        Err(e) => {
            tracing::error!(
                "route_request returned Err for request_id: {}, error: {}",
                request_id,
                e
            );
            let conn = connection_state.read();
            let response =
                ResponseEnvelope::error(request_id, e.error_code().to_string(), e.to_string());
            let _ = conn.send_response(response);
        }
    }
}
