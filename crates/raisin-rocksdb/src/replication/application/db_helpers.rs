//! Common RocksDB operation helpers
//!
//! This module provides utilities for common RocksDB operations with consistent
//! error handling and logging.

use raisin_error::Result;
use rocksdb::DB;
use serde::Serialize;
use std::sync::Arc;

/// Serialize and write a value to RocksDB with error handling
///
/// Uses `rmp_serde::to_vec_named` for named serialization to maintain compatibility
/// with the repository layer's serialization format.
pub fn serialize_and_write<T: Serialize>(
    db: &Arc<DB>,
    cf: &rocksdb::ColumnFamily,
    key: Vec<u8>,
    value: &T,
    context: &str,
) -> Result<()> {
    let serialized = rmp_serde::to_vec_named(value).map_err(|e| {
        tracing::error!(
            context = context,
            error = %e,
            "Failed to serialize value"
        );
        raisin_error::Error::storage(format!("Serialization error: {}", e))
    })?;

    db.put_cf(cf, key, serialized).map_err(|e| {
        tracing::error!(
            context = context,
            error = %e,
            "Failed to write to RocksDB"
        );
        raisin_error::Error::storage(e.to_string())
    })?;

    Ok(())
}

/// Serialize and write a value using standard serialization (not named)
///
/// Some types like simple structs don't need named serialization
pub fn serialize_and_write_compact<T: Serialize>(
    db: &Arc<DB>,
    cf: &rocksdb::ColumnFamily,
    key: Vec<u8>,
    value: &T,
    context: &str,
) -> Result<()> {
    let serialized = rmp_serde::to_vec(value).map_err(|e| {
        tracing::error!(
            context = context,
            error = %e,
            "Failed to serialize value"
        );
        raisin_error::Error::storage(format!("Serialization error: {}", e))
    })?;

    db.put_cf(cf, key, serialized).map_err(|e| {
        tracing::error!(
            context = context,
            error = %e,
            "Failed to write to RocksDB"
        );
        raisin_error::Error::storage(e.to_string())
    })?;

    Ok(())
}

/// Delete all keys with a given prefix from a column family
///
/// This is commonly used for schema deletions where we need to remove all
/// versions of a schema entity.
pub fn delete_with_prefix(
    db: &Arc<DB>,
    cf: &rocksdb::ColumnFamily,
    prefix: &[u8],
    context: &str,
) -> Result<usize> {
    let keys_to_delete: Vec<Vec<u8>> = db
        .iterator_cf(
            cf,
            rocksdb::IteratorMode::From(prefix, rocksdb::Direction::Forward),
        )
        .take_while(|result| {
            if let Ok((key, _)) = result {
                key.starts_with(prefix)
            } else {
                false
            }
        })
        .filter_map(|result| result.ok().map(|(key, _)| key.to_vec()))
        .collect();

    let count = keys_to_delete.len();

    for key in keys_to_delete {
        db.delete_cf(cf, key).map_err(|e| {
            tracing::error!(
                context = context,
                error = %e,
                "Failed to delete key"
            );
            raisin_error::Error::storage(e.to_string())
        })?;
    }

    tracing::debug!(context = context, count = count, "Deleted keys with prefix");

    Ok(count)
}

/// Write a simple key-value pair (for non-serialized data like strings)
pub fn write_raw(
    db: &Arc<DB>,
    cf: &rocksdb::ColumnFamily,
    key: Vec<u8>,
    value: &[u8],
    context: &str,
) -> Result<()> {
    db.put_cf(cf, key, value).map_err(|e| {
        tracing::error!(
            context = context,
            error = %e,
            "Failed to write raw value to RocksDB"
        );
        raisin_error::Error::storage(e.to_string())
    })
}

/// Delete a key from RocksDB
pub fn delete_key(
    db: &Arc<DB>,
    cf: &rocksdb::ColumnFamily,
    key: Vec<u8>,
    context: &str,
) -> Result<()> {
    db.delete_cf(cf, key).map_err(|e| {
        tracing::error!(
            context = context,
            error = %e,
            "Failed to delete key from RocksDB"
        );
        raisin_error::Error::storage(e.to_string())
    })
}
