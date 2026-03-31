//! RocksDB implementation of Identity storage.
//!
//! This module provides persistent storage for user identities in the
//! pluggable authentication system. Identities are global per tenant
//! and can have multiple authentication providers linked.
//!
//! # Key Format
//!
//! - Identity: `{tenant}\0identities\0{identity_id}`
//! - Email Index: `{tenant}\0identity_email\0{email}`
//!
//! # Replication
//!
//! Identity operations are replicated across cluster nodes via the OpLog.
//! Uses LWW (Last-Write-Wins) semantics for conflict resolution.

mod crud;
mod password;
mod replication;
#[cfg(test)]
mod tests;

use crate::{cf, keys, replication::OperationCapture};
use raisin_error::Result;
use rocksdb::DB;
use std::sync::Arc;

/// RocksDB-backed identity repository.
///
/// Provides CRUD operations for identities with email index for lookups.
/// All mutations are captured to the operation log for cluster replication.
pub struct IdentityRepository {
    pub(super) db: Arc<DB>,
    pub(super) operation_capture: Arc<OperationCapture>,
}

impl IdentityRepository {
    /// Create a new identity repository.
    pub fn new(db: Arc<DB>, operation_capture: Arc<OperationCapture>) -> Self {
        Self {
            db,
            operation_capture,
        }
    }

    /// Get the column family handle for identities.
    pub(super) fn cf_identities(&self) -> Result<&rocksdb::ColumnFamily> {
        self.db.cf_handle(cf::IDENTITIES).ok_or_else(|| {
            raisin_error::Error::storage(format!("Column family '{}' not found", cf::IDENTITIES))
        })
    }

    /// Get the column family handle for email index.
    pub(super) fn cf_email_index(&self) -> Result<&rocksdb::ColumnFamily> {
        self.db.cf_handle(cf::IDENTITY_EMAIL_INDEX).ok_or_else(|| {
            raisin_error::Error::storage(format!(
                "Column family '{}' not found",
                cf::IDENTITY_EMAIL_INDEX
            ))
        })
    }
}
