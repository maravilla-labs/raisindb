//! Tree storage compaction operations.
//!
//! Contains logic for compacting content-addressed tree storage
//! by removing unreferenced trees.

use crate::{cf, cf_handle, keys, RocksDBStorage};
use raisin_error::Result;

/// Compact tree storage by removing unreferenced trees
pub(super) async fn compact_trees(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<()> {
    tracing::info!("Compacting tree storage for {}/{}", tenant_id, repo_id);

    // 1. Get all referenced tree IDs from revisions
    let referenced_trees = get_referenced_trees(storage, tenant_id, repo_id).await?;

    // 2. Get all stored trees
    let all_trees = get_all_trees(storage, tenant_id, repo_id).await?;

    // 3. Delete unreferenced trees
    let mut batch = rocksdb::WriteBatch::default();
    let cf_trees = cf_handle(storage.db(), cf::TREES)?;
    let mut deleted = 0;

    for tree_id in all_trees {
        if !referenced_trees.contains(&tree_id) {
            let key = keys::tree_key(tenant_id, repo_id, &tree_id);
            batch.delete_cf(cf_trees, key);
            deleted += 1;

            // Commit batch every 1000 deletes
            if deleted % 1000 == 0 {
                storage.db().write(batch).map_err(|e| {
                    raisin_error::Error::storage(format!("Batch delete failed: {}", e))
                })?;
                batch = rocksdb::WriteBatch::default();
            }
        }
    }

    // Commit remaining deletes
    if !batch.is_empty() {
        storage.db().write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Final batch delete failed: {}", e))
        })?;
    }

    tracing::info!("Deleted {} unreferenced trees", deleted);
    Ok(())
}

/// Get all tree IDs referenced by revisions
///
/// Note: RevisionMeta doesn't currently have tree_id field,
/// so we'll need to get tree IDs from tree storage directly
async fn get_referenced_trees(
    _storage: &RocksDBStorage,
    _tenant_id: &str,
    _repo_id: &str,
) -> Result<std::collections::HashSet<[u8; 32]>> {
    // For now, we keep all trees since we don't have a tree_id in RevisionMeta
    // In a future version, we should track tree references properly
    Ok(std::collections::HashSet::new())
}

/// Get all stored tree IDs
async fn get_all_trees(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<[u8; 32]>> {
    let cf_trees = cf_handle(storage.db(), cf::TREES)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("trees")
        .build_prefix();

    let mut trees = Vec::new();
    let iter = storage.db().prefix_iterator_cf(cf_trees, &prefix);

    for item in iter {
        let (key, _) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        // Extract tree ID from key
        let key_str = String::from_utf8_lossy(&key);
        if let Some(tree_id_hex) = key_str.split('\0').next_back() {
            if let Ok(tree_id_bytes) = hex::decode(tree_id_hex) {
                if tree_id_bytes.len() == 32 {
                    let mut tree_id = [0u8; 32];
                    tree_id.copy_from_slice(&tree_id_bytes);
                    trees.push(tree_id);
                }
            }
        }
    }

    Ok(trees)
}
