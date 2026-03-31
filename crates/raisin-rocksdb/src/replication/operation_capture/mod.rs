//! Operation capture for CRDT replication
//!
//! This module captures database operations and logs them to the operation log
//! for replication to other cluster nodes and offline clients.
//!
//! # Submodules
//!
//! - `core` - Core capture logic and state management
//! - `node_ops` - Convenience methods for node operations
//! - `schema_ops` - Convenience methods for schema and registry operations

mod core;
mod node_ops;
mod schema_ops;

#[cfg(test)]
mod tests;

pub use self::core::OperationCapture;
