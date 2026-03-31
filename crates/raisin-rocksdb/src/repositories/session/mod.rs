//! RocksDB implementation of Session storage.
//!
//! This module provides persistent storage for authentication sessions.
//! Sessions track active user logins and are linked to identities.
//!
//! # Key Format
//!
//! - Session: `{tenant}\0sessions\0{session_id}`
//! - Identity Sessions Index: `{tenant}\0identity_sessions\0{identity_id}\0{session_id}`
//!
//! # Replication
//!
//! Session operations are replicated across cluster nodes via the OpLog.

mod crud;
mod replication;
#[cfg(test)]
mod tests;

use crate::{cf, keys, replication::OperationCapture};
use raisin_error::Result;
use raisin_models::auth::Session;
use rocksdb::DB;
use std::sync::Arc;

/// RocksDB-backed session repository.
///
/// Provides CRUD operations for sessions with identity index for lookups.
/// All mutations are captured to the operation log for cluster replication.
pub struct SessionRepository {
    pub(super) db: Arc<DB>,
    pub(super) operation_capture: Arc<OperationCapture>,
}

impl SessionRepository {
    /// Create a new session repository.
    pub fn new(db: Arc<DB>, operation_capture: Arc<OperationCapture>) -> Self {
        Self {
            db,
            operation_capture,
        }
    }

    /// Get the column family handle for sessions.
    pub(super) fn cf_sessions(&self) -> Result<&rocksdb::ColumnFamily> {
        self.db.cf_handle(cf::SESSIONS).ok_or_else(|| {
            raisin_error::Error::storage(format!("Column family '{}' not found", cf::SESSIONS))
        })
    }
}
