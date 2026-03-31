// SPDX-License-Identifier: BSL-1.1

//! Core connection state management.
//!
//! Holds per-connection state including authentication, channels,
//! concurrency control, and transaction context.

use crate::protocol::{EventMessage, ResponseEnvelope, SubscriptionFilters};
use dashmap::DashMap;
use parking_lot::{Mutex, RwLock};
use raisin_models::auth::AuthContext;
use raisin_storage::transactional::TransactionalContext;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Semaphore};
use uuid::Uuid;

use super::errors::{AcquireError, SendError, TransactionError};

/// Connection state for a single WebSocket connection
#[derive(Clone)]
pub struct ConnectionState {
    /// Unique connection ID
    pub connection_id: String,

    /// Tenant ID for this connection
    pub tenant_id: String,

    /// Optional repository scope
    pub repository: Option<String>,

    /// Authenticated user ID (after authentication)
    pub user_id: Option<String>,

    /// Auth context for identity users (for row-level security)
    auth_context: Option<AuthContext>,

    /// Session-level branch override (from USE BRANCH / SET app.branch)
    session_branch: Arc<RwLock<Option<String>>>,

    /// Map of request ID to response sender
    pending_requests: Arc<DashMap<String, oneshot::Sender<ResponseEnvelope>>>,

    /// Map of subscription ID to filters
    pub(super) subscriptions: Arc<DashMap<String, SubscriptionFilters>>,

    /// Index from filter hash to subscription ID (for deduplication)
    pub(super) filter_index: Arc<DashMap<String, String>>,

    /// Semaphore for limiting concurrent operations per connection
    operation_semaphore: Arc<Semaphore>,

    /// Channel for sending responses back to the WebSocket
    response_tx: Arc<RwLock<Option<mpsc::UnboundedSender<ResponseEnvelope>>>>,

    /// Channel for sending events back to the WebSocket
    event_tx: Arc<RwLock<Option<mpsc::UnboundedSender<EventMessage>>>>,

    /// Flow control credits (for backpressure)
    credits: Arc<RwLock<u32>>,

    /// Active transaction context (if a transaction is in progress)
    transaction_context: Arc<Mutex<Option<Arc<dyn TransactionalContext>>>>,

    /// Anonymous JWT token (for HTTP API calls when using anonymous access)
    anonymous_token: Option<String>,
}

impl ConnectionState {
    /// Create a new connection state
    pub fn new(
        tenant_id: String,
        repository: Option<String>,
        max_concurrent_ops: usize,
        initial_credits: u32,
    ) -> Self {
        Self {
            connection_id: Uuid::new_v4().to_string(),
            tenant_id,
            repository,
            user_id: None,
            auth_context: None,
            session_branch: Arc::new(RwLock::new(None)),
            pending_requests: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
            filter_index: Arc::new(DashMap::new()),
            operation_semaphore: Arc::new(Semaphore::new(max_concurrent_ops)),
            response_tx: Arc::new(RwLock::new(None)),
            event_tx: Arc::new(RwLock::new(None)),
            credits: Arc::new(RwLock::new(initial_credits)),
            transaction_context: Arc::new(Mutex::new(None)),
            anonymous_token: None,
        }
    }

    /// Set the authenticated user ID
    pub fn set_user_id(&mut self, user_id: String) {
        self.user_id = Some(user_id);
    }

    /// Set the auth context for identity users
    pub fn set_auth_context(&mut self, auth_context: AuthContext) {
        self.auth_context = Some(auth_context);
    }

    /// Get the auth context
    pub fn auth_context(&self) -> Option<&AuthContext> {
        self.auth_context.as_ref()
    }

    /// Set the anonymous JWT token (for HTTP API calls)
    pub fn set_anonymous_token(&mut self, token: Option<String>) {
        self.anonymous_token = token;
    }

    /// Get the anonymous JWT token
    pub fn anonymous_token(&self) -> Option<&String> {
        self.anonymous_token.as_ref()
    }

    /// Set the session-level branch (from USE BRANCH / SET app.branch)
    pub fn set_session_branch(&self, branch: String) {
        *self.session_branch.write() = Some(branch);
    }

    /// Get the session-level branch
    pub fn session_branch(&self) -> Option<String> {
        self.session_branch.read().clone()
    }

    /// Clear the session-level branch
    pub fn clear_session_branch(&self) {
        *self.session_branch.write() = None;
    }

    /// Check if the connection is authenticated
    pub fn is_authenticated(&self) -> bool {
        self.user_id.is_some()
    }

    /// Set the response channel
    pub fn set_response_channel(&self, tx: mpsc::UnboundedSender<ResponseEnvelope>) {
        *self.response_tx.write() = Some(tx);
    }

    /// Set the event channel
    pub fn set_event_channel(&self, tx: mpsc::UnboundedSender<EventMessage>) {
        *self.event_tx.write() = Some(tx);
    }

    /// Register a pending request
    pub fn register_request(&self, request_id: String, tx: oneshot::Sender<ResponseEnvelope>) {
        self.pending_requests.insert(request_id, tx);
    }

    /// Complete a pending request with a response
    pub fn complete_request(&self, request_id: &str, response: ResponseEnvelope) -> bool {
        if let Some((_, tx)) = self.pending_requests.remove(request_id) {
            let _ = tx.send(response);
            true
        } else {
            false
        }
    }

    /// Send a response directly to the WebSocket (for streaming)
    pub fn send_response(&self, response: ResponseEnvelope) -> Result<(), SendError> {
        tracing::info!(
            "send_response() called - request_id: {}, status: {:?}",
            response.request_id,
            response.status
        );

        let tx = self.response_tx.read().clone();
        if let Some(tx) = tx {
            tracing::info!(
                "Sending response to channel - request_id: {}",
                response.request_id
            );
            match tx.send(response.clone()) {
                Ok(_) => {
                    tracing::info!(
                        "Response sent successfully to channel - request_id: {}",
                        response.request_id
                    );
                    Ok(())
                }
                Err(_) => {
                    tracing::error!(
                        "Failed to send response: channel closed - request_id: {}",
                        response.request_id
                    );
                    Err(SendError::ChannelClosed)
                }
            }
        } else {
            tracing::error!(
                "Failed to send response: channel not set - request_id: {}",
                response.request_id
            );
            Err(SendError::ChannelNotSet)
        }
    }

    /// Send an event to the WebSocket
    pub fn send_event(&self, event: EventMessage) -> Result<(), SendError> {
        let tx = self.event_tx.read().clone();
        if let Some(tx) = tx {
            tx.send(event).map_err(|_| SendError::ChannelClosed)?;
            Ok(())
        } else {
            Err(SendError::ChannelNotSet)
        }
    }

    /// Acquire a permit for an operation (for concurrency control)
    pub async fn acquire_permit(&self) -> Result<tokio::sync::SemaphorePermit<'_>, AcquireError> {
        self.operation_semaphore
            .acquire()
            .await
            .map_err(|_| AcquireError::SemaphoreClosed)
    }

    /// Try to acquire a permit without blocking
    pub fn try_acquire_permit(&self) -> Result<tokio::sync::SemaphorePermit<'_>, AcquireError> {
        self.operation_semaphore
            .try_acquire()
            .map_err(|_| AcquireError::NoPermitsAvailable)
    }

    /// Get available credits
    pub fn get_credits(&self) -> u32 {
        *self.credits.read()
    }

    /// Try to consume credits (for backpressure)
    pub fn try_consume_credits(&self, amount: u32) -> bool {
        let mut credits = self.credits.write();
        if *credits >= amount {
            *credits -= amount;
            true
        } else {
            false
        }
    }

    /// Add credits (when client sends credit grant)
    pub fn add_credits(&self, amount: u32) {
        let mut credits = self.credits.write();
        *credits = credits.saturating_add(amount);
    }

    /// Get the operation semaphore (for acquiring permits with longer lifetime)
    pub fn get_operation_semaphore(&self) -> Arc<Semaphore> {
        Arc::clone(&self.operation_semaphore)
    }

    /// Set the active transaction context
    pub fn set_transaction_context(
        &mut self,
        ctx: Box<dyn TransactionalContext>,
    ) -> Result<(), TransactionError> {
        let mut guard = self.transaction_context.lock();
        if guard.is_some() {
            return Err(TransactionError::AlreadyActive);
        }
        *guard = Some(Arc::from(ctx));
        Ok(())
    }

    /// Get a reference to the active transaction context
    pub fn get_transaction_context(&self) -> Option<Arc<dyn TransactionalContext>> {
        let guard = self.transaction_context.lock();
        guard.as_ref().map(Arc::clone)
    }

    /// Take the active transaction context (for commit/rollback)
    pub fn take_transaction_context(&mut self) -> Option<Arc<dyn TransactionalContext>> {
        self.transaction_context.lock().take()
    }

    /// Check if there's an active transaction
    pub fn has_active_transaction(&self) -> bool {
        self.transaction_context.lock().is_some()
    }

    /// Cleanup connection state (called on disconnect)
    pub fn cleanup(&self) {
        self.pending_requests.clear();
        self.subscriptions.clear();
        self.filter_index.clear();
        *self.response_tx.write() = None;
        *self.event_tx.write() = None;
        *self.transaction_context.lock() = None;
    }
}
