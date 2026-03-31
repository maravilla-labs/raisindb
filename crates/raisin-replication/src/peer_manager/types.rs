//! Types for peer connection management
//!
//! Defines the connection pool, peer connection state, status, and error types.

use std::collections::VecDeque;
use std::time::Instant;

use tokio::net::TcpStream;

use crate::config::PeerConfig;
use crate::tcp_protocol::ProtocolError;

/// Connection pool for a single peer
pub(super) struct ConnectionPool {
    /// Available connections (idle streams ready to use)
    available: VecDeque<TcpStream>,

    /// Maximum number of connections allowed
    max_connections: usize,

    /// Current total connections (available + in-use)
    pub(super) current_count: usize,
}

impl ConnectionPool {
    pub(super) fn new(max_connections: usize) -> Self {
        Self {
            available: VecDeque::new(),
            max_connections,
            current_count: 0,
        }
    }

    /// Try to acquire an available connection, or None if pool is empty
    pub(super) fn try_acquire(&mut self) -> Option<TcpStream> {
        self.available.pop_front()
    }

    /// Return a connection to the pool
    pub(super) fn release(&mut self, stream: TcpStream) {
        if self.available.len() < self.max_connections {
            self.available.push_back(stream);
        } else {
            // Pool is full, drop the connection
            self.current_count = self.current_count.saturating_sub(1);
        }
    }

    /// Add a new connection to the pool
    pub(super) fn add_connection(&mut self, stream: TcpStream) {
        if self.current_count < self.max_connections {
            self.available.push_back(stream);
            self.current_count += 1;
        }
    }

    /// Clear all connections from the pool
    pub(super) fn clear(&mut self) {
        self.available.clear();
        self.current_count = 0;
    }

    /// Check if we can create more connections
    pub(super) fn can_create_more(&self) -> bool {
        self.current_count < self.max_connections
    }

    /// Increment connection count (when creating new connection)
    pub(super) fn increment_count(&mut self) {
        self.current_count += 1;
    }

    /// Decrement connection count when a connection is dropped
    pub(super) fn drop_connection(&mut self) {
        self.current_count = self.current_count.saturating_sub(1);
    }
}

/// Represents a connection to a single peer
pub(in crate::peer_manager) struct PeerConnection {
    /// Peer configuration
    pub(super) peer_config: PeerConfig,

    /// Connection pool for this peer
    pub(super) pool: ConnectionPool,

    /// Last successful heartbeat
    pub(super) last_heartbeat: Instant,

    /// Connection state
    pub(super) state: ConnectionState,

    /// Number of consecutive connection failures
    pub(super) failed_attempts: usize,

    /// Last error (if any)
    pub(super) last_error: Option<String>,

    /// Maximum connections per peer (from config)
    pub(super) max_connections: usize,
}

/// Connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not yet connected
    Disconnected,

    /// Currently connecting
    Connecting,

    /// Connected and ready
    Connected,

    /// Temporarily failed, will retry
    Failed,

    /// Permanently disabled
    Disabled,
}

/// Peer status information
#[derive(Debug, Clone)]
pub struct PeerStatus {
    pub peer_id: String,
    pub state: ConnectionState,
    pub last_heartbeat: Instant,
    pub failed_attempts: usize,
    pub last_error: Option<String>,
}

/// Peer manager errors
#[derive(Debug, thiserror::Error)]
pub enum PeerManagerError {
    #[error("Peer not found: {0}")]
    PeerNotFound(String),

    #[error("Not connected to peer: {0}")]
    NotConnected(String),

    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Handshake failed: {0}")]
    Handshake(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Operation timed out")]
    Timeout,
}

impl From<ProtocolError> for PeerManagerError {
    fn from(e: ProtocolError) -> Self {
        PeerManagerError::Protocol(e.to_string())
    }
}
