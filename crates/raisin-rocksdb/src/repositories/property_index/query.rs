//! Property index query operations
//!
//! NOTE: File slightly exceeds 300 lines (~348) because the 4 query functions
//! share MVCC-aware prefix scan logic. Further splitting would create excessive
//! fragmentation without meaningful cohesion gains.
//!
//! Provides exact-match lookups, limited lookups, counting, and
//! property existence queries against the property index.

use super::helpers::{extract_node_id_from_key, is_tombstone};
use crate::repositories::nodes::hash_property_value;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use rocksdb::DB;
use std::sync::Arc;

pub(super) async fn find_by_property(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    property_value: &PropertyValue,
    published_only: bool,
) -> Result<Vec<String>> {
    let value_hash = hash_property_value(property_value);
    let tag = if published_only { "prop_pub" } else { "prop" };

    tracing::debug!(
        "🔍 PropertyIndex find_by_property: property='{}', value={:?}, value_hash='{}', tenant='{}', repo='{}', branch='{}', workspace='{}', tag='{}'",
        property_name,
        property_value,
        value_hash,
        tenant_id,
        repo_id,
        branch,
        workspace,
        tag
    );

    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(property_name)
        .push(&value_hash)
        .build_prefix();

    let prefix_str = String::from_utf8_lossy(&prefix);
    tracing::debug!("🔍 Search prefix: {:?}", prefix_str);

    let cf = cf_handle(db, cf::PROPERTY_INDEX)?;
    let prefix_clone = prefix.clone();
    let iter = db.prefix_iterator_cf(cf, prefix);

    // Use HashSet to deduplicate node IDs
    // (same node may appear at multiple revisions)
    let mut node_ids = std::collections::HashSet::new();
    // Track tombstoned node_ids for MVCC - tombstones at newer revisions should
    // prevent older entries from resurrecting deleted nodes
    let mut tombstoned_node_ids = std::collections::HashSet::new();
    let mut keys_found = 0;
    let mut keys_skipped_tombstone = 0;
    let mut keys_skipped_prefix = 0;

    for item in iter {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        keys_found += 1;

        let key_str = String::from_utf8_lossy(&key);
        tracing::trace!(
            "  Found key #{}: {:?}, value_len={}",
            keys_found,
            key_str,
            value.len()
        );

        // Verify key actually starts with our prefix
        if !key.starts_with(&prefix_clone) {
            keys_skipped_prefix += 1;
            tracing::trace!("  SKIP: key doesn't match prefix");
            break;
        }

        // Extract node_id from key (last component after final null byte)
        // Note: Cannot use split('\0') because HLC's 16 binary bytes may contain 0x00
        let node_id = match extract_node_id_from_key(&key) {
            Some(id) => id,
            None => {
                tracing::trace!("  SKIP: empty or missing node_id");
                continue;
            }
        };

        // Skip tombstones and track them for MVCC
        // Tombstones are either empty values or the explicit TOMBSTONE marker (b"T")
        if value.is_empty() || is_tombstone(&value) {
            keys_skipped_tombstone += 1;
            tracing::trace!("  SKIP: tombstone");
            // Track tombstoned node_id to prevent resurrection by older entries
            tombstoned_node_ids.insert(node_id);
            continue;
        }

        // Skip entries for node_ids that have been tombstoned at a newer revision
        if tombstoned_node_ids.contains(&node_id) {
            tracing::trace!("  SKIP: node_id already tombstoned");
            continue;
        }

        tracing::debug!("  ✓ Extracted node_id: {}", node_id);
        node_ids.insert(node_id);
    }

    tracing::debug!(
        "🔍 PropertyIndex search complete: keys_found={}, keys_skipped_tombstone={}, keys_skipped_prefix={}, unique_node_ids={}",
        keys_found,
        keys_skipped_tombstone,
        keys_skipped_prefix,
        node_ids.len()
    );

    Ok(node_ids.into_iter().collect())
}

pub(super) async fn find_by_property_with_limit(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    property_value: &PropertyValue,
    published_only: bool,
    limit: Option<usize>,
) -> Result<Vec<String>> {
    let value_hash = hash_property_value(property_value);
    let tag = if published_only { "prop_pub" } else { "prop" };

    tracing::debug!(
        "🔍 PropertyIndex find_by_property_with_limit: property='{}', value_hash='{}', limit={:?}",
        property_name,
        value_hash,
        limit
    );

    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(property_name)
        .push(&value_hash)
        .build_prefix();

    let cf = cf_handle(db, cf::PROPERTY_INDEX)?;
    let prefix_clone = prefix.clone();
    let iter = db.prefix_iterator_cf(cf, prefix);

    // Use HashSet to deduplicate node IDs
    let mut node_ids = std::collections::HashSet::new();
    // Track tombstoned node_ids for MVCC - tombstones at newer revisions should
    // prevent older entries from resurrecting deleted nodes
    let mut tombstoned_node_ids = std::collections::HashSet::new();

    for item in iter {
        // Early termination if limit reached
        if let Some(lim) = limit {
            if node_ids.len() >= lim {
                break;
            }
        }

        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Verify key actually starts with our prefix
        if !key.starts_with(&prefix_clone) {
            break;
        }

        // Extract node_id from key (last component after final null byte)
        // Note: Cannot use split('\0') because HLC's 16 binary bytes may contain 0x00
        let node_id = match extract_node_id_from_key(&key) {
            Some(id) => id,
            None => continue,
        };

        // Skip tombstones and track them for MVCC
        // Tombstones are either empty values or the explicit TOMBSTONE marker (b"T")
        if value.is_empty() || is_tombstone(&value) {
            // Mark this node_id as tombstoned so older entries won't resurrect it
            tombstoned_node_ids.insert(node_id);
            continue;
        }

        // Skip entries for node_ids that have been tombstoned at a newer revision
        // (since we iterate newest-first due to descending revision encoding)
        if tombstoned_node_ids.contains(&node_id) {
            continue;
        }

        node_ids.insert(node_id);
    }

    tracing::debug!(
        "🔍 PropertyIndex search complete: unique_node_ids={}, limit={:?}",
        node_ids.len(),
        limit
    );

    Ok(node_ids.into_iter().collect())
}

pub(super) async fn count_by_property(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    property_value: &PropertyValue,
    published_only: bool,
) -> Result<usize> {
    let value_hash = hash_property_value(property_value);
    let tag = if published_only { "prop_pub" } else { "prop" };

    tracing::debug!(
        "🔢 PropertyIndex count_by_property: property='{}', value_hash='{}'",
        property_name,
        value_hash
    );

    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(property_name)
        .push(&value_hash)
        .build_prefix();

    let cf = cf_handle(db, cf::PROPERTY_INDEX)?;
    let prefix_clone = prefix.clone();
    let iter = db.prefix_iterator_cf(cf, prefix);

    // Use HashSet to deduplicate node IDs (same node may appear at multiple revisions)
    let mut unique_nodes = std::collections::HashSet::new();
    // Track tombstoned node_ids for MVCC
    let mut tombstoned_node_ids = std::collections::HashSet::new();

    for item in iter {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Verify key actually starts with our prefix
        if !key.starts_with(&prefix_clone) {
            break;
        }

        // Extract node_id from key (last component after final null byte)
        // Note: Cannot use split('\0') because HLC's 16 binary bytes may contain 0x00
        let node_id = match extract_node_id_from_key(&key) {
            Some(id) => id,
            None => continue,
        };

        // Skip tombstones and track them for MVCC
        if value.is_empty() || is_tombstone(&value) {
            tombstoned_node_ids.insert(node_id);
            continue;
        }

        // Skip entries for node_ids that have been tombstoned at a newer revision
        if tombstoned_node_ids.contains(&node_id) {
            continue;
        }

        unique_nodes.insert(node_id);
    }

    let count = unique_nodes.len();
    tracing::debug!("🔢 PropertyIndex count complete: count={}", count);

    Ok(count)
}

pub(super) async fn find_nodes_with_property(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    property_name: &str,
    published_only: bool,
) -> Result<Vec<String>> {
    let tag = if published_only { "prop_pub" } else { "prop" };

    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(tag)
        .push(property_name)
        .build_prefix();

    let cf = cf_handle(db, cf::PROPERTY_INDEX)?;
    let prefix_clone = prefix.clone();
    let iter = db.prefix_iterator_cf(cf, prefix);

    // Use HashSet to deduplicate node IDs
    // (same node may appear at multiple revisions with different values)
    let mut node_ids = std::collections::HashSet::new();
    // Track tombstoned node_ids for MVCC
    let mut tombstoned_node_ids = std::collections::HashSet::new();

    for item in iter {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Verify key actually starts with our prefix
        if !key.starts_with(&prefix_clone) {
            break;
        }

        // Extract node_id from key (last component after final null byte)
        // Note: Cannot use split('\0') because HLC's 16 binary bytes may contain 0x00
        let node_id = match extract_node_id_from_key(&key) {
            Some(id) => id,
            None => continue,
        };

        // Skip tombstones and track them for MVCC
        if value.is_empty() || is_tombstone(&value) {
            tombstoned_node_ids.insert(node_id);
            continue;
        }

        // Skip entries for node_ids that have been tombstoned at a newer revision
        if tombstoned_node_ids.contains(&node_id) {
            continue;
        }

        node_ids.insert(node_id);
    }

    Ok(node_ids.into_iter().collect())
}
