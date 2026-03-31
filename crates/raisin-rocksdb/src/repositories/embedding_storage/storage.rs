//! RocksDB-backed embedding storage implementation.
//!
//! Stores embedding vectors with revision awareness in the `embeddings` column family.

use crate::{cf, cf_handle};
use raisin_embeddings::{EmbeddingData, EmbeddingStorage};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

/// RocksDB-backed embedding storage
///
/// Stores embedding vectors with revision awareness in the `embeddings` CF.
///
/// # Key Format (Multi-Model)
///
/// New format: `{tenant}\0{repo}\0{branch}\0{workspace}\0{embedder_hash:11}\0{kind:1}\0{source_id}\0{chunk_idx:04}\0{revision:HLC:16bytes}`
/// Legacy format: `{tenant}\0{repo}\0{branch}\0{workspace}\0{node_id}\0{revision:HLC:16bytes}`
///
/// The new format includes:
/// - embedder_hash: 11-character base64url hash identifying the embedding model
/// - kind: Single character ('T' for text, 'I' for image)
/// - source_id: Node ID or asset ID
/// - chunk_idx: 4-digit zero-padded chunk index (e.g., "0000", "0001")
///
/// Revisions are encoded as full HLC (16 bytes) in descending ordering,
/// preserving both timestamp and counter components. Latest revisions sort first.
#[derive(Clone)]
pub struct RocksDBEmbeddingStorage {
    db: Arc<DB>,
}

impl RocksDBEmbeddingStorage {
    /// Create a new RocksDB embedding storage
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Build embedding key with new multi-model format
    fn embedding_key_v2(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        embedder_hash: &str,
        kind: char,
        source_id: &str,
        chunk_idx: usize,
        revision: &HLC,
    ) -> Vec<u8> {
        let mut key = Vec::new();
        key.extend_from_slice(tenant_id.as_bytes());
        key.push(0);
        key.extend_from_slice(repo_id.as_bytes());
        key.push(0);
        key.extend_from_slice(branch.as_bytes());
        key.push(0);
        key.extend_from_slice(workspace_id.as_bytes());
        key.push(0);
        key.extend_from_slice(embedder_hash.as_bytes());
        key.push(0);
        key.push(kind as u8);
        key.push(0);
        key.extend_from_slice(source_id.as_bytes());
        key.push(0);
        // 4-digit zero-padded chunk index
        key.extend_from_slice(format!("{:04}", chunk_idx).as_bytes());
        key.push(0);
        // Encode full HLC in descending order (latest first)
        key.extend_from_slice(&revision.encode_descending());

        key
    }

    /// Build embedding key with legacy format (for backward compatibility)
    fn embedding_key_legacy(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        revision: &HLC,
    ) -> Vec<u8> {
        let mut key = Vec::new();
        key.extend_from_slice(tenant_id.as_bytes());
        key.push(0);
        key.extend_from_slice(repo_id.as_bytes());
        key.push(0);
        key.extend_from_slice(branch.as_bytes());
        key.push(0);
        key.extend_from_slice(workspace_id.as_bytes());
        key.push(0);
        key.extend_from_slice(node_id.as_bytes());
        key.push(0);

        // Encode full HLC in descending order (latest first)
        // Uses bitwise NOT on both timestamp and counter components
        key.extend_from_slice(&revision.encode_descending());

        key
    }

    /// Parse key to extract components
    /// Returns (embedder_hash, kind, source_id, chunk_idx, is_legacy)
    fn parse_key(key: &[u8]) -> Option<(String, char, String, usize, bool)> {
        let key_str = String::from_utf8_lossy(key);
        let parts: Vec<&str> = key_str.split('\0').collect();

        // Check if this is a legacy key (6 parts) or new key (9+ parts)
        if parts.len() >= 9 {
            // New format: tenant, repo, branch, workspace, embedder_hash, kind, source_id, chunk_idx, revision
            let embedder_hash = parts[4].to_string();
            let kind_char = parts[5].chars().next()?;
            let source_id = parts[6].to_string();
            let chunk_idx = parts[7].parse().ok()?;
            Some((embedder_hash, kind_char, source_id, chunk_idx, false))
        } else if parts.len() >= 6 {
            // Legacy format: tenant, repo, branch, workspace, node_id, revision
            // Return a synthetic embedder_hash to indicate legacy
            let node_id = parts[4].to_string();
            Some(("legacy".to_string(), 'T', node_id, 0, true))
        } else {
            None
        }
    }

    /// Build prefix for source (all chunks, all revisions) with new format
    fn source_prefix_v2(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        embedder_hash: &str,
        kind: char,
        source_id: &str,
    ) -> Vec<u8> {
        let mut prefix = Vec::new();
        prefix.extend_from_slice(tenant_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(repo_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(branch.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(workspace_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(embedder_hash.as_bytes());
        prefix.push(0);
        prefix.push(kind as u8);
        prefix.push(0);
        prefix.extend_from_slice(source_id.as_bytes());
        prefix.push(0);
        prefix
    }

    /// Build prefix for node (all revisions) - legacy format
    fn node_prefix_legacy(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
    ) -> Vec<u8> {
        let mut prefix = Vec::new();
        prefix.extend_from_slice(tenant_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(repo_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(branch.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(workspace_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(node_id.as_bytes());
        prefix.push(0);
        prefix
    }

    /// Build prefix for workspace (all embeddings)
    fn workspace_prefix(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
    ) -> Vec<u8> {
        let mut prefix = Vec::new();
        prefix.extend_from_slice(tenant_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(repo_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(branch.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(workspace_id.as_bytes());
        prefix.push(0);
        prefix
    }

    /// Serialize embedding data
    fn serialize(data: &EmbeddingData) -> Result<Vec<u8>> {
        rmp_serde::to_vec_named(data).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize embedding: {}", e))
        })
    }

    /// Deserialize embedding data
    fn deserialize(bytes: &[u8]) -> Result<EmbeddingData> {
        rmp_serde::from_slice(bytes).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to deserialize embedding: {}", e))
        })
    }
}

impl EmbeddingStorage for RocksDBEmbeddingStorage {
    fn store_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        revision: &HLC,
        data: &EmbeddingData,
    ) -> Result<()> {
        let cf = cf_handle(&self.db, cf::EMBEDDINGS)?;

        // Use new key format with embedder_id
        let embedder_hash = data.embedder_id.to_key_hash();
        let kind_char = data.embedding_kind.to_key_char();
        let key = Self::embedding_key_v2(
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            &embedder_hash,
            kind_char,
            &data.source_id,
            data.chunk_index,
            revision,
        );
        let value = Self::serialize(data)?;

        self.db.put_cf(cf, key, value).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to store embedding: {}", e))
        })?;

        tracing::debug!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            workspace_id = %workspace_id,
            source_id = %data.source_id,
            chunk = %data.chunk_index,
            revision = %revision,
            dims = data.vector.len(),
            embedder = %embedder_hash,
            "Stored embedding"
        );

        Ok(())
    }

    fn get_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        revision: Option<&HLC>,
    ) -> Result<Option<EmbeddingData>> {
        let cf = cf_handle(&self.db, cf::EMBEDDINGS)?;

        // Try legacy format first for backward compatibility
        if let Some(rev) = revision {
            // Get specific revision - try legacy key
            let legacy_key =
                Self::embedding_key_legacy(tenant_id, repo_id, branch, workspace_id, node_id, rev);

            let value = self.db.get_cf(cf, &legacy_key).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to get embedding: {}", e))
            })?;

            match value {
                Some(bytes) => return Ok(Some(Self::deserialize(&bytes)?)),
                None => {
                    // Fall through to check new format (scan prefix)
                    // This is necessary because we don't know the embedder_hash without scanning
                }
            }
        }

        // Get latest revision (first in prefix scan due to descending order)
        // Try legacy format first
        let prefix = Self::node_prefix_legacy(tenant_id, repo_id, branch, workspace_id, node_id);
        let mut iter = self.db.prefix_iterator_cf(cf, &prefix);

        if let Some(result) = iter.next() {
            let (key, value) = result.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate embeddings: {}", e))
            })?;
            if key.starts_with(&prefix) {
                return Ok(Some(Self::deserialize(&value)?));
            }
        }

        // Try v2 format: scan workspace prefix and filter by source_id
        let ws_prefix = Self::workspace_prefix(tenant_id, repo_id, branch, workspace_id);
        let iter = self.db.prefix_iterator_cf(cf, &ws_prefix);

        for result in iter {
            let (key, value) = result.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate embeddings: {}", e))
            })?;

            if !key.starts_with(&ws_prefix) {
                break;
            }

            if let Some((_, _, source_id, _, _)) = Self::parse_key(&key) {
                if source_id == node_id {
                    // If a specific revision was requested, verify it matches
                    if let Some(rev) = revision {
                        if key.len() >= 16 {
                            let key_rev_bytes = &key[key.len() - 16..];
                            if key_rev_bytes == rev.encode_descending().as_slice() {
                                return Ok(Some(Self::deserialize(&value)?));
                            }
                        }
                    } else {
                        // No specific revision, return latest (first match due to descending order)
                        return Ok(Some(Self::deserialize(&value)?));
                    }
                }
            }
        }

        Ok(None)
    }

    fn delete_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        revision: Option<&HLC>,
    ) -> Result<()> {
        let cf = cf_handle(&self.db, cf::EMBEDDINGS)?;

        let rev_desc = revision.map(|r| r.encode_descending());

        // Collect keys to delete
        let mut batch = WriteBatch::default();

        if let Some(rev) = revision {
            // Delete specific revision - try legacy key directly
            let legacy_key =
                Self::embedding_key_legacy(tenant_id, repo_id, branch, workspace_id, node_id, rev);
            self.db.delete_cf(cf, &legacy_key).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to delete embedding: {}", e))
            })?;
        }

        // Scan legacy prefix for matching keys
        let legacy_prefix =
            Self::node_prefix_legacy(tenant_id, repo_id, branch, workspace_id, node_id);
        let iter = self.db.prefix_iterator_cf(cf, &legacy_prefix);
        for result in iter {
            let (key, _) = result.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate embeddings: {}", e))
            })?;
            if !key.starts_with(&legacy_prefix) {
                break;
            }
            if let Some(ref rev_bytes) = rev_desc {
                if key.len() >= 16 && &key[key.len() - 16..] == rev_bytes.as_slice() {
                    batch.delete_cf(cf, key);
                }
            } else {
                batch.delete_cf(cf, key);
            }
        }

        // Scan workspace prefix for v2 keys matching this source_id
        let ws_prefix = Self::workspace_prefix(tenant_id, repo_id, branch, workspace_id);
        let iter = self.db.prefix_iterator_cf(cf, &ws_prefix);
        for result in iter {
            let (key, _) = result.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate embeddings: {}", e))
            })?;
            if !key.starts_with(&ws_prefix) {
                break;
            }
            if let Some((_, _, source_id, _, is_legacy)) = Self::parse_key(&key) {
                if is_legacy {
                    continue; // Already handled above
                }
                if source_id == node_id {
                    if let Some(ref rev_bytes) = rev_desc {
                        if key.len() >= 16 && &key[key.len() - 16..] == rev_bytes.as_slice() {
                            batch.delete_cf(cf, key);
                        }
                    } else {
                        batch.delete_cf(cf, key);
                    }
                }
            }
        }

        if !batch.is_empty() {
            self.db.write(batch).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to delete embeddings: {}", e))
            })?;
        }

        tracing::debug!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            workspace_id = %workspace_id,
            node_id = %node_id,
            revision = ?revision,
            "Deleted embedding(s)"
        );

        Ok(())
    }

    fn list_embeddings(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
    ) -> Result<Vec<(String, HLC)>> {
        let cf = cf_handle(&self.db, cf::EMBEDDINGS)?;
        let prefix = Self::workspace_prefix(tenant_id, repo_id, branch, workspace_id);
        let iter = self.db.prefix_iterator_cf(cf, &prefix);

        let mut results = Vec::new();
        let mut last_node_id: Option<String> = None;

        for result in iter {
            let (key, _) = result.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate embeddings: {}", e))
            })?;

            // Verify key matches prefix
            if !key.starts_with(&prefix) {
                break;
            }

            // Parse key: {tenant}\0{repo}\0{branch}\0{workspace}\0{node_id}\0{revision}
            let key_str = String::from_utf8_lossy(&key);
            let parts: Vec<&str> = key_str.split('\0').collect();

            if parts.len() >= 5 {
                let node_id = parts[4].to_string();

                // Only include each node once (latest revision due to descending order)
                if last_node_id.as_ref() != Some(&node_id) {
                    // Extract HLC from last 16 bytes
                    if key.len() >= 16 {
                        let hlc_bytes = &key[key.len() - 16..];
                        let revision = HLC::decode_descending(hlc_bytes).map_err(|e| {
                            raisin_error::Error::storage(format!("Invalid HLC encoding: {}", e))
                        })?;

                        results.push((node_id.clone(), revision));
                        last_node_id = Some(node_id);
                    }
                }
            }
        }

        Ok(results)
    }
}
