//! Orphaned property index cleanup.
//!
//! Scans property index entries and removes any that reference
//! nodes which no longer exist, fixing inconsistencies from
//! partial write failures.

use crate::{cf, cf_handle, keys, RocksDBStorage};
use raisin_error::Result;
use rocksdb::WriteBatch;

use super::helpers::scan_nodes;

/// Statistics from orphaned property index cleanup
#[derive(Debug, Clone, Default)]
pub struct OrphanedIndexCleanupStats {
    /// Total index entries scanned
    pub entries_scanned: usize,
    /// Number of orphaned entries found (pointing to non-existent nodes)
    pub orphaned_found: usize,
    /// Number of orphaned entries deleted
    pub orphaned_deleted: usize,
    /// Number of errors during cleanup
    pub errors: usize,
    /// Duration in milliseconds
    pub duration_ms: u64,
}

/// Clean up orphaned property index entries
///
/// This function scans all property index entries and removes any that reference
/// nodes that no longer exist. This fixes issues where:
/// - Direct CRUD operations failed between atomic batch write and revision indexing
/// - Nodes were deleted but property indexes weren't cleaned up properly
///
/// Returns statistics about the cleanup operation.
pub async fn cleanup_orphaned_property_indexes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Result<OrphanedIndexCleanupStats> {
    let start = std::time::Instant::now();
    let mut stats = OrphanedIndexCleanupStats::default();

    tracing::info!(
        "🧹 Starting orphaned property index cleanup for {}/{}/{}/{}",
        tenant_id,
        repo_id,
        branch,
        workspace
    );

    // Get column family handle for property index
    let cf_prop = cf_handle(storage.db(), cf::PROPERTY_INDEX)?;

    // Build the set of valid node IDs first (more efficient than per-entry lookups)
    let valid_node_ids = {
        let nodes = scan_nodes(storage, tenant_id, repo_id, branch, workspace).await?;
        nodes
            .into_iter()
            .map(|n| n.id)
            .collect::<std::collections::HashSet<_>>()
    };
    tracing::info!("🧹 Found {} valid nodes in workspace", valid_node_ids.len());

    // Scan property indexes for both draft and published
    let mut batch = WriteBatch::default();
    let mut batch_count = 0;

    for tag in &["prop", "prop_pub"] {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push(tag)
            .build_prefix();

        let iter = storage.db().prefix_iterator_cf(cf_prop, &prefix);

        for item in iter {
            let (key, value) = match item {
                Ok(kv) => kv,
                Err(e) => {
                    tracing::warn!("🧹 Iterator error: {}", e);
                    stats.errors += 1;
                    continue;
                }
            };

            stats.entries_scanned += 1;

            // Skip tombstones
            if value.as_ref() == b"T" {
                continue;
            }

            // Extract node_id from the index entry
            // The value typically contains the node_id, or we need to parse it from the key
            let node_id = if !value.is_empty() && value.as_ref() != b"" {
                // Value contains the node_id
                String::from_utf8_lossy(&value).to_string()
            } else {
                // Node ID is embedded in the key - extract from last component
                // Key format: tenant\0repo\0branch\0workspace\0prop\0property_name\0value\0revision\0node_id
                let key_str = String::from_utf8_lossy(&key);
                key_str.rsplit('\0').next().unwrap_or("").to_string()
            };

            if node_id.is_empty() {
                continue;
            }

            // Check if this node exists
            if !valid_node_ids.contains(&node_id) {
                // Orphaned entry - node doesn't exist
                stats.orphaned_found += 1;

                // Log first few orphaned entries for debugging
                if stats.orphaned_found <= 10 {
                    tracing::debug!(
                        "🧹 Found orphaned index entry: node_id='{}' (entry #{})",
                        node_id,
                        stats.orphaned_found
                    );
                }

                // Delete the orphaned entry
                batch.delete_cf(cf_prop, &key);
                stats.orphaned_deleted += 1;
                batch_count += 1;

                // Commit batch every 1000 deletes
                if batch_count >= 1000 {
                    storage.db().write(batch).map_err(|e| {
                        raisin_error::Error::storage(format!("Batch delete failed: {}", e))
                    })?;
                    batch = WriteBatch::default();
                    batch_count = 0;
                    tracing::debug!(
                        "🧹 Committed batch, deleted {} orphaned entries so far",
                        stats.orphaned_deleted
                    );
                }
            }
        }
    }

    // Commit remaining deletes
    if !batch.is_empty() {
        storage.db().write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Final batch delete failed: {}", e))
        })?;
    }

    stats.duration_ms = start.elapsed().as_millis() as u64;

    tracing::info!(
        "🧹 Orphaned index cleanup complete: scanned={}, orphaned_found={}, deleted={}, errors={}, duration={}ms",
        stats.entries_scanned,
        stats.orphaned_found,
        stats.orphaned_deleted,
        stats.errors,
        stats.duration_ms
    );

    Ok(stats)
}
