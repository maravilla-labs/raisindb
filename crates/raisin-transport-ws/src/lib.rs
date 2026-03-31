// SPDX-License-Identifier: BSL-1.1

// TODO(v0.2): Update deprecated API usages to new methods and clean up
#![allow(deprecated)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

//! WebSocket transport for RaisinDB
//!
//! This crate provides a WebSocket-based transport layer for RaisinDB,
//! enabling real-time bidirectional communication with clients.
//!
//! ## Features
//!
//! - **MessagePack serialization** for efficient binary communication
//! - **JWT authentication** for secure connections
//! - **Async request/response** pattern with request ID tracking
//! - **Event subscriptions** with flexible filtering
//! - **Streaming responses** with backpressure control
//! - **Concurrency control** per-connection and globally
//! - **Connection pooling** pattern for efficient resource usage
//!
//! ## Example
//!
//! ```rust,no_run
//! use raisin_transport_ws::{WsConfig, WsState, websocket_handler};
//! use axum::{Router, routing::get};
//! use std::sync::Arc;
//!
//! # async fn example() {
//! // Create WebSocket state
//! // let state = Arc::new(WsState::new(...));
//!
//! // Create router with WebSocket endpoint
//! // let app = Router::new()
//! //     .route("/ws", get(websocket_handler))
//! //     .with_state(state);
//! # }
//! ```

pub mod auth;
pub mod connection;
pub mod error;
pub mod event_handler;
pub mod handler;
pub mod handlers;
pub mod protocol;
pub mod registry;

// Re-exports
pub use auth::{Claims, JwtAuthService, TokenType};
pub use connection::ConnectionState;
pub use error::WsError;
pub use event_handler::WsEventHandler;
pub use handler::{websocket_handler, WsConfig, WsState};
pub use protocol::{EventMessage, RequestEnvelope, RequestType, ResponseEnvelope, ResponseStatus};
pub use registry::ConnectionRegistry;

// Version info
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
