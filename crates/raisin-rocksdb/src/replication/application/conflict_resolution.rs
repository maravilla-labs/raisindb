//! Last-Write-Wins (LWW) conflict resolution helpers
//!
//! This module provides utilities for resolving conflicts between replicated operations
//! using a Last-Write-Wins strategy based on timestamps.

use raisin_error::Result;
use rocksdb::DB;
use serde::de::DeserializeOwned;
use std::sync::Arc;

/// Trait for types that have a timestamp for LWW comparison
pub trait HasTimestamp {
    fn timestamp(&self) -> chrono::DateTime<chrono::Utc>;
}

/// Check if an update should be applied based on LWW conflict resolution
///
/// Returns `Ok(true)` if the update should be applied (doesn't exist or newer timestamp)
/// Returns `Ok(false)` if the update should be skipped (existing is newer)
///
/// # Arguments
/// * `db` - RocksDB instance
/// * `cf` - Column family handle
/// * `key` - Key to check
/// * `new_value` - New value to potentially apply
pub fn should_apply_lww<T>(
    db: &Arc<DB>,
    cf: &rocksdb::ColumnFamily,
    key: &[u8],
    new_value: &T,
) -> Result<bool>
where
    T: HasTimestamp + DeserializeOwned,
{
    match db.get_cf(cf, key) {
        Ok(Some(bytes)) => {
            // Entry exists - check timestamp for LWW
            match rmp_serde::from_slice::<T>(&bytes) {
                Ok(existing) => {
                    let should_update = new_value.timestamp() >= existing.timestamp();
                    if !should_update {
                        tracing::debug!(
                            "Skipping update - existing is newer: {:?} >= {:?}",
                            existing.timestamp(),
                            new_value.timestamp()
                        );
                    }
                    Ok(should_update)
                }
                Err(_) => {
                    // Corrupted data, allow overwrite
                    tracing::warn!("Found corrupted data, allowing overwrite");
                    Ok(true)
                }
            }
        }
        Ok(None) => Ok(true), // Doesn't exist, apply it
        Err(e) => {
            tracing::error!("Failed to check existing value: {}", e);
            Err(raisin_error::Error::storage(e.to_string()))
        }
    }
}

/// Helper to check LWW for types with a `last_seen` field
///
/// This uses a closure to extract the timestamp field, allowing flexibility
/// for different struct layouts without requiring trait implementations
pub fn should_apply_by_last_seen<T, F>(
    db: &Arc<DB>,
    cf: &rocksdb::ColumnFamily,
    key: &[u8],
    new_last_seen: chrono::DateTime<chrono::Utc>,
    get_last_seen: F,
) -> Result<bool>
where
    T: DeserializeOwned,
    F: Fn(&T) -> chrono::DateTime<chrono::Utc>,
{
    match db.get_cf(cf, key) {
        Ok(Some(bytes)) => match rmp_serde::from_slice::<T>(&bytes) {
            Ok(existing) => {
                let existing_last_seen = get_last_seen(&existing);
                let should_update = new_last_seen >= existing_last_seen;
                if !should_update {
                    tracing::debug!(
                        "Skipping update - existing is newer: {:?} >= {:?}",
                        existing_last_seen,
                        new_last_seen
                    );
                }
                Ok(should_update)
            }
            Err(_) => {
                tracing::warn!("Found corrupted data, allowing overwrite");
                Ok(true)
            }
        },
        Ok(None) => Ok(true),
        Err(e) => {
            tracing::error!("Failed to check existing value: {}", e);
            Err(raisin_error::Error::storage(e.to_string()))
        }
    }
}
