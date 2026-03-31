//! Binary TCP protocol for peer-to-peer replication
//!
//! This module defines the wire protocol used for direct database-to-database
//! synchronization. The protocol is storage-agnostic and can be used with any
//! storage backend that implements the operation log interface.
//!
//! ## Protocol Design
//!
//! - **Transport**: Raw TCP sockets (typically port 9001)
//! - **Serialization**: MessagePack for efficient binary encoding
//! - **Connection**: Persistent connections with heartbeat
//! - **Flow**: Bidirectional (both push and pull operations)
//!
//! ## Message Flow
//!
//! ```text
//! Client                          Server
//!   |                               |
//!   |-- Hello ---------------------->|
//!   |<--------------------- HelloAck-|
//!   |                               |
//!   |-- PullOperations ------------->|
//!   |<------------- OperationBatch--|
//!   |                               |
//!   |-- Ack ------------------------>|
//!   |                               |
//!   |-- PushOperations ------------->|
//!   |<------------------------- Ack--|
//!   |                               |
//!   |-- Ping ----------------------->|
//!   |<------------------------ Pong--|
//!   |                               |
//! ```

mod constants;
mod error;
mod file_transfer;
mod message;
mod message_impl;

#[cfg(test)]
mod tests;

// Re-export all public items to maintain the same public API
pub use constants::{
    DEFAULT_BATCH_SIZE, DEFAULT_MAX_PARALLEL_FILES, MAX_MESSAGE_SIZE, PROTOCOL_VERSION,
};
pub use error::{ErrorCode, ProtocolError};
pub use file_transfer::{IndexFileInfo, SstFileInfo, TransferStatus};
pub use message::ReplicationMessage;
