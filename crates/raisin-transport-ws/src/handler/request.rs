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
