//! RocksDB checkpoint management for cluster catch-up
//!
//! This module provides functionality to create atomic snapshots of the RocksDB
//! database and transfer them to other nodes for fast cluster catch-up.

mod manager;
mod receiver;
#[cfg(test)]
mod tests;

pub use manager::{CheckpointManager, CheckpointMetadata};
pub use receiver::CheckpointReceiver;
