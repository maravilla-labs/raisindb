//! CRDT replication module
//!
//! This module provides configuration and coordination for peer-to-peer
//! replication in distributed RaisinDB clusters.

pub mod peer_config;
pub mod sync_coordinator;

pub use peer_config::{PeerConfig, PeerRegistry, RetryConfig};
pub use sync_coordinator::SyncCoordinator;
