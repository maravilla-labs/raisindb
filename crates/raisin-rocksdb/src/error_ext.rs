//! Error extension utilities for RocksDB operations

use raisin_error::Result;

/// Extension trait for converting RocksDB errors to our error type
pub trait ResultExt<T> {
    fn rocksdb_err(self) -> Result<T>;
}

impl<T> ResultExt<T> for std::result::Result<T, rocksdb::Error> {
    fn rocksdb_err(self) -> Result<T> {
        self.map_err(|e| raisin_error::Error::storage(e.to_string()))
    }
}

impl<T> ResultExt<T> for std::result::Result<T, serde_json::Error> {
    fn rocksdb_err(self) -> Result<T> {
        self.map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))
    }
}
