//! Shared helper functions for async index rebuilding.
//!
//! Provides node scanning, index clearing, key prefix deletion,
//! revision lookup, and reference extraction utilities.

use crate::{cf, cf_handle, keys, RocksDBStorage};
use raisin_error::Result;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;
use std::collections::HashMap;

/// Scan all nodes in a workspace
pub(super) async fn scan_nodes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<Vec<Node>> {
    let cf_nodes = cf_handle(storage.db(), cf::NODES)?;
    let prefix = keys::workspace_prefix(tenant_id, repo_id, branch, workspace);

    let mut nodes = Vec::new();
    let iter = storage.db().prefix_iterator_cf(cf_nodes, &prefix);

    for item in iter {
        let (key, value) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        // Only process keys with "nodes" component
        let key_str = String::from_utf8_lossy(&key);
        if !key_str.contains("\0nodes\0") {
            continue;
        }

        // Skip empty values and tombstones (single byte 'T' = 84)
        if value.is_empty() || value.as_ref() == b"T" {
            continue;
        }

        match rmp_serde::from_slice::<Node>(&value) {
            Ok(node) => nodes.push(node),
            Err(e) => {
                tracing::warn!("Failed to deserialize node: {}", e);
            }
        }
    }

    Ok(nodes)
}

/// Clear all path indexes for a workspace
pub(super) async fn clear_path_indexes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<()> {
    let cf_path = cf_handle(storage.db(), cf::PATH_INDEX)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("path")
        .build_prefix();

    delete_with_prefix(storage, cf_path, &prefix)?;
    Ok(())
}

/// Clear all property indexes for a workspace
pub(super) async fn clear_property_indexes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<()> {
    let cf_prop = cf_handle(storage.db(), cf::PROPERTY_INDEX)?;

    // Clear both draft and published property indexes
    for tag in &["prop", "prop_pub"] {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push(tag)
            .build_prefix();

        delete_with_prefix(storage, cf_prop, &prefix)?;
    }

    Ok(())
}

/// Clear all reference indexes for a workspace
pub(super) async fn clear_reference_indexes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<()> {
    let cf_ref = cf_handle(storage.db(), cf::REFERENCE_INDEX)?;

    // Clear both forward and reverse, draft and published
    for tag in &["ref", "ref_pub", "ref_rev", "ref_rev_pub"] {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push(tag)
            .build_prefix();

        delete_with_prefix(storage, cf_ref, &prefix)?;
    }

    Ok(())
}

/// Clear all child order indexes for a workspace
pub(super) async fn clear_order_indexes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<()> {
    let cf_order = cf_handle(storage.db(), cf::ORDER_INDEX)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("order")
        .build_prefix();

    delete_with_prefix(storage, cf_order, &prefix)?;
    Ok(())
}

/// Delete all keys with a given prefix
fn delete_with_prefix(
    storage: &RocksDBStorage,
    cf: &rocksdb::ColumnFamily,
    prefix: &[u8],
) -> Result<()> {
    let iter = storage.db().prefix_iterator_cf(cf, prefix);
    let mut batch = WriteBatch::default();
    let mut count = 0;

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        batch.delete_cf(cf, key);
        count += 1;

        // Commit every 10000 deletes
        if count % 10000 == 0 {
            storage
                .db()
                .write(batch)
                .map_err(|e| raisin_error::Error::storage(format!("Batch delete failed: {}", e)))?;
            batch = WriteBatch::default();
        }
    }

    // Commit remaining deletes
    if !batch.is_empty() {
        storage.db().write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Final batch delete failed: {}", e))
        })?;
    }

    tracing::debug!("Deleted {} keys with prefix", count);
    Ok(())
}

/// Get current revision for a branch
pub(super) async fn get_current_revision(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<raisin_hlc::HLC> {
    // Try to get branch metadata
    let cf_branches = cf_handle(storage.db(), cf::BRANCHES)?;
    let branch_key = keys::branch_key(tenant_id, repo_id, branch);

    match storage
        .db()
        .get_cf(cf_branches, branch_key)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to get branch: {}", e)))?
    {
        Some(data) => {
            let branch_meta: raisin_context::Branch =
                rmp_serde::from_slice(&data).map_err(|e| {
                    raisin_error::Error::storage(format!("Failed to deserialize branch: {}", e))
                })?;
            Ok(branch_meta.head)
        }
        None => {
            // No branch metadata, use initial HLC
            Ok(raisin_hlc::HLC::new(0, 0))
        }
    }
}

/// Extract all references from node properties
pub(super) fn extract_references(
    properties: &HashMap<String, raisin_models::nodes::properties::PropertyValue>,
) -> Vec<(String, raisin_models::nodes::properties::RaisinReference)> {
    let mut references = Vec::new();

    for (key, value) in properties {
        extract_references_recursive(key, value, &mut references);
    }

    references
}

/// Recursively extract references from a property value
fn extract_references_recursive(
    path: &str,
    value: &raisin_models::nodes::properties::PropertyValue,
    results: &mut Vec<(String, raisin_models::nodes::properties::RaisinReference)>,
) {
    use raisin_models::nodes::properties::PropertyValue;

    match value {
        PropertyValue::Reference(r) => {
            results.push((path.to_string(), r.clone()));
        }
        PropertyValue::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let item_path = format!("{}.{}", path, i);
                extract_references_recursive(&item_path, item, results);
            }
        }
        PropertyValue::Object(map) => {
            for (key, val) in map {
                let nested_path = format!("{}.{}", path, key);
                extract_references_recursive(&nested_path, val, results);
            }
        }
        _ => {}
    }
}
