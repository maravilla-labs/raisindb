//! TCP connection management for replication peers
//!
//! This module provides connection pooling, heartbeat monitoring, and automatic
//! reconnection for peer-to-peer replication.

mod connection;
mod heartbeat;
mod io;
#[cfg(test)]
mod tests;
pub mod types;

pub use types::{ConnectionState, PeerManagerError, PeerStatus};

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::config::{ConnectionConfig, RetryConfig};

use types::PeerConnection;

/// Callback type for connection state changes
type ConnectionCallback = Arc<dyn Fn(String) + Send + Sync>;

/// Manages persistent TCP connections to replication peers
pub struct PeerManager {
    /// This node's cluster ID
    pub(crate) cluster_node_id: String,

    /// Map of peer_id -> PeerConnection
    pub(in crate::peer_manager) peers: Arc<RwLock<HashMap<String, Arc<Mutex<PeerConnection>>>>>,

    /// Configuration
    pub(super) config: ConnectionConfig,

    /// Retry configuration
    retry_config: RetryConfig,

    /// Callback invoked when a peer connection is established (uses Mutex for interior mutability)
    pub(super) on_connected: Mutex<Option<ConnectionCallback>>,
}

impl PeerManager {
    /// Create a new peer manager
    pub fn new(
        cluster_node_id: String,
        config: ConnectionConfig,
        retry_config: RetryConfig,
    ) -> Self {
        Self {
            cluster_node_id,
            peers: Arc::new(RwLock::new(HashMap::new())),
            config,
            retry_config,
            on_connected: Mutex::new(None),
        }
    }

    /// Set callback to be invoked when a peer connects
    pub async fn set_on_connected<F>(&self, callback: F)
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        *self.on_connected.lock().await = Some(Arc::new(callback));
    }
}
