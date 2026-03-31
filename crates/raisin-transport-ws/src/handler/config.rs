// SPDX-License-Identifier: BSL-1.1

//! WebSocket configuration and path parameter types.

/// Configuration for WebSocket connections
#[derive(Clone)]
pub struct WsConfig {
    /// Maximum concurrent operations per connection
    pub max_concurrent_ops: usize,

    /// Initial flow control credits per connection
    pub initial_credits: u32,

    /// JWT secret for authentication
    pub jwt_secret: String,

    /// Whether authentication is required (default: true)
    pub require_auth: bool,

    /// Global concurrency limit (across all connections)
    pub global_concurrency_limit: Option<usize>,

    /// Whether anonymous access is enabled globally.
    /// When true, unauthenticated connections will be auto-authenticated
    /// as the "anonymous" user with the "anonymous" role permissions.
    pub anonymous_enabled: bool,

    /// Development mode — allows insecure JWT fallbacks.
    /// NEVER enable in production.
    pub dev_mode: bool,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            max_concurrent_ops: 600,
            initial_credits: 500,
            jwt_secret: "change_me_in_production".to_string(),
            require_auth: true,
            global_concurrency_limit: Some(1000),
            anonymous_enabled: false,
            dev_mode: false,
        }
    }
}

/// Path parameters for WebSocket routes
#[derive(Debug, serde::Deserialize)]
pub struct WsPathParams {
    pub tenant_id: String,
    pub repository: Option<String>,
}
