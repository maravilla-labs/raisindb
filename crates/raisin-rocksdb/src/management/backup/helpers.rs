//! Helper functions for backup and restore

use crate::{cf, cf_handle, keys, RocksDBStorage};
use raisin_error::Result;
use std::path::Path;

/// Calculate total size of a directory recursively
pub(super) fn calculate_directory_size(path: &Path) -> Result<u64> {
    let mut total_size = 0u64;

    if path.is_file() {
        return Ok(std::fs::metadata(path)
            .map_err(|e| {
                raisin_error::Error::storage(format!("Failed to get file metadata: {}", e))
            })?
            .len());
    }

    for entry in std::fs::read_dir(path)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to read directory: {}", e)))?
    {
        let entry = entry.map_err(|e| {
            raisin_error::Error::storage(format!("Failed to read directory entry: {}", e))
        })?;

        let metadata = entry
            .metadata()
            .map_err(|e| raisin_error::Error::storage(format!("Failed to get metadata: {}", e)))?;

        if metadata.is_file() {
            total_size += metadata.len();
        } else if metadata.is_dir() {
            total_size += calculate_directory_size(&entry.path())?;
        }
    }

    Ok(total_size)
}

/// List all repositories for a tenant
pub(super) async fn list_repositories(
    storage: &RocksDBStorage,
    tenant_id: &str,
) -> Result<Vec<String>> {
    let cf_registry = cf_handle(storage.db(), cf::REGISTRY)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push("repos")
        .build_prefix();

    let mut repos = Vec::new();
    let iter = storage.db().prefix_iterator_cf(cf_registry, &prefix);

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        let key_str = String::from_utf8_lossy(&key);
        let parts: Vec<&str> = key_str.split('\0').collect();
        if parts.len() >= 3 {
            repos.push(parts[2].to_string());
        }
    }

    Ok(repos)
}
