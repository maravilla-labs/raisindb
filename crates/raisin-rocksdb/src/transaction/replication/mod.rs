//! Replication operation capture for RocksDB transactions
//!
//! This module contains all the logic for capturing operations for CRDT replication, including:
//! - Operation capture internal logic
//! - Tracked change processing
//! - Node change operations (create, delete, update)
//! - Property, relation, and metadata change capture
//! - ApplyRevision operation building

mod capture;
mod node_changes;
mod relations;
