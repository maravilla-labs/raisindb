//! Node revision compaction operations.
//!
//! Contains logic for compacting individual node revisions based on retention policies.

use super::RevisionRetentionPolicy;
use crate::{cf, cf_handle, keys, RocksDBStorage};
use raisin_error::Result;
use raisin_hlc::HLC;

/// Compact revisions for a single node
pub(super) async fn compact_node_revisions(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    node_id: &str,
    policy: &RevisionRetentionPolicy,
) -> Result<()> {
    use rocksdb::WriteBatch;

    // Get all revisions for this node
    let revisions = get_node_revisions(storage, tenant_id, repo_id, node_id).await?;

    if revisions.is_empty() {
        return Ok(());
    }

    // Determine which revisions to keep
    let to_keep = match policy {
        RevisionRetentionPolicy::KeepLatest(n) => {
            // Keep the N most recent (revisions are sorted descending)
            revisions.iter().take(*n).map(|r| r.0).collect::<Vec<_>>()
        }
        RevisionRetentionPolicy::KeepSince(duration) => {
            let cutoff = chrono::Utc::now()
                - chrono::Duration::from_std(*duration).map_err(|e| {
                    raisin_error::Error::invalid_state(format!(
                        "Invalid duration for retention policy: {}",
                        e
                    ))
                })?;
            revisions
                .iter()
                .filter(|(_, timestamp)| timestamp > &cutoff)
                .map(|r| r.0)
                .collect::<Vec<_>>()
        }
        RevisionRetentionPolicy::KeepAll => return Ok(()),
    };

    // Delete old revisions
    let mut batch = WriteBatch::default();
    let cf_nodes = cf_handle(storage.db(), cf::NODES)?;

    for (revision, _) in revisions {
        if !to_keep.contains(&revision) {
            // Delete this revision from all branches and workspaces
            delete_node_revision(storage, tenant_id, repo_id, node_id, &revision, &mut batch)
                .await?;
        }
    }

    if !batch.is_empty() {
        storage.db().write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to delete old revisions: {}", e))
        })?;
    }

    Ok(())
}

/// Get all revisions for a node across all branches/workspaces
async fn get_node_revisions(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    node_id: &str,
) -> Result<Vec<(HLC, chrono::DateTime<chrono::Utc>)>> {
    let cf_nodes = cf_handle(storage.db(), cf::NODES)?;

    // Scan with prefix to find all revisions
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .build_prefix();

    let mut revisions = Vec::new();
    let iter = storage.db().prefix_iterator_cf(cf_nodes, &prefix);

    for item in iter {
        let (key, value) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        let key_str = String::from_utf8_lossy(&key);

        // Check if this key is for our node
        if !key_str.contains(&format!("\0nodes\0{}\0", node_id)) {
            continue;
        }

        // Extract revision from key
        let parts: Vec<&str> = key_str.split('\0').collect();
        if let Some(rev_bytes) = parts.last() {
            if let Ok(rev) = keys::decode_descending_revision(rev_bytes.as_bytes()) {
                // Get timestamp from node data
                if !value.is_empty() {
                    if let Ok(node) = rmp_serde::from_slice::<raisin_models::nodes::Node>(&value) {
                        let timestamp = node.updated_at.unwrap_or_else(chrono::Utc::now);
                        revisions.push((rev, timestamp));
                    }
                }
            }
        }
    }

    // Sort by revision (descending - newest first)
    revisions.sort_by(|a, b| b.0.cmp(&a.0));

    Ok(revisions)
}

/// Delete a specific node revision
async fn delete_node_revision(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    node_id: &str,
    revision: &HLC,
    batch: &mut rocksdb::WriteBatch,
) -> Result<()> {
    let cf_nodes = cf_handle(storage.db(), cf::NODES)?;

    // We need to scan for all instances of this node at this revision
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .build_prefix();

    let iter = storage.db().prefix_iterator_cf(cf_nodes, &prefix);

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        let key_str = String::from_utf8_lossy(&key);

        // Check if this is our node at the target revision
        if key_str.contains(&format!("\0nodes\0{}\0", node_id)) {
            if let Some(rev_part) = key_str.split('\0').next_back() {
                if let Ok(rev) = keys::decode_descending_revision(rev_part.as_bytes()) {
                    if &rev == revision {
                        batch.delete_cf(cf_nodes, &key);
                    }
                }
            }
        }
    }

    Ok(())
}
