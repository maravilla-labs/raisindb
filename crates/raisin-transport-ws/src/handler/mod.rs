// SPDX-License-Identifier: BSL-1.1

//! WebSocket handler for RaisinDB
//!
//! This module implements the main WebSocket connection handler,
//! managing the lifecycle of connections and message routing.

mod auth_token;
mod config;
mod request;
mod socket;
mod state;

// Re-export public API
pub use config::{WsConfig, WsPathParams};
pub use state::{websocket_handler, WsState};
