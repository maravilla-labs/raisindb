//! RocksDB-backed embedding storage implementation.
//!
//! Stores embedding vectors with revision awareness in the `embeddings` column family.

use crate::{cf, cf_handle};
use raisin_embeddings::{EmbeddingData, EmbeddingStorage, StoredIndexEntry};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{WriteBatch, DB};
use std::collections::HashMap;
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

    /// Prefix covering EVERY workspace under `{tenant}/{repo}/{branch}`.
    ///
    /// Same layout as `workspace_prefix`, stopping one segment earlier. Used by
    /// `list_workspaces` so administrative operations can discover the
    /// workspaces they must cover instead of guessing one by literal.
    fn branch_prefix(tenant_id: &str, repo_id: &str, branch: &str) -> Vec<u8> {
        let mut prefix = Vec::new();
        prefix.extend_from_slice(tenant_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(repo_id.as_bytes());
        prefix.push(0);
        prefix.extend_from_slice(branch.as_bytes());
        prefix.push(0);
        prefix
    }

    /// Prefix covering ONE chunk of one source: everything up to and including
    /// the chunk-index segment, so what remains is only the revision.
    ///
    /// Note what a "chunk" is in this CF, because it is NOT what it is in the
    /// HNSW index: here every chunk of a document shares the document's SOURCE
    /// ID and is distinguished by the `{chunk_idx:04}` key segment, while
    /// `{node}#{n}` is the HNSW vocabulary only. Confusing the two is how a
    /// sweep aimed at chunk rows matches either nothing or everything.
    #[allow(clippy::too_many_arguments)]
    fn chunk_prefix_v2(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        embedder_hash: &str,
        kind: char,
        source_id: &str,
        chunk_idx: usize,
    ) -> Vec<u8> {
        let mut prefix = Self::source_prefix_v2(
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            embedder_hash,
            kind,
            source_id,
        );
        prefix.extend_from_slice(format!("{:04}", chunk_idx).as_bytes());
        prefix.push(0);
        prefix
    }

    /// Iterate forward from `prefix`. The CALLER stops on the first key that no
    /// longer starts with it — every loop in this file already does.
    ///
    /// ONE scan, rather than eight hand-written `prefix_iterator_cf` calls with
    /// eight hand-written `starts_with` guards. That matters here more than it
    /// usually would, because this CF is scanned at FOUR different prefix
    /// lengths — branch, workspace, source, chunk — and the guard is what makes
    /// each of them correct.
    ///
    /// An explicit seek rather than `DB::prefix_iterator_cf` on purpose.
    /// `prefix_iterator_cf` sets `prefix_same_as_start`, which is only
    /// meaningful against a configured prefix extractor. `cf::EMBEDDINGS` has
    /// none today (`prefix_transform::custom_prefix_extractor` returns `Some`
    /// for `ORDERED_CHILDREN` and `SPATIAL_INDEX` only), so the two forms are
    /// equivalent RIGHT NOW — and would stop being equivalent the day someone
    /// adds an extractor to this CF for bloom filtering. At that moment every
    /// one of these scans is silently cut short at a prefix boundary the
    /// extractor chose, and the symptoms are all of the invisible kind: a
    /// chunked source reporting one chunk (which is `VECTOR_OF` deciding it is
    /// unambiguous), a workspace listing that skips workspaces, a delete sweep
    /// that leaves orphans behind.
    ///
    /// `prefix_transform.rs` documents the inverse hazard — a short prefix on a
    /// CF that DOES have an extractor — and prescribes exactly this: seek, and
    /// compare the prefix yourself.
    ///
    /// Note this is NOT the cause of the known NULL `SELECT embedding` column
    /// (`get_embedding` returning `None` for nodes that have a stored vector).
    /// That reproduces with either form and is still open.
    fn scan_from<'a>(
        &'a self,
        cf: &impl rocksdb::AsColumnFamilyRef,
        prefix: &[u8],
    ) -> rocksdb::DBIteratorWithThreadMode<'a, DB> {
        self.db.iterator_cf(
            cf,
            rocksdb::IteratorMode::From(prefix, rocksdb::Direction::Forward),
        )
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

    /// Read ONE chunk row, addressed exactly.
    ///
    /// `EmbeddingStorage::get_embedding` cannot do this. It takes no embedder,
    /// so it scans the WHOLE workspace prefix looking for a matching source id
    /// and answers with whichever embedder's row it meets first; and it takes no
    /// chunk index, so for a chunked document it always answers with chunk 0. A
    /// caller that knows which model and which chunk it is asking about — the
    /// embedding job does, it just resolved one and split the text itself — gets
    /// the exact key instead.
    #[allow(clippy::too_many_arguments)]
    pub fn get_chunk(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        embedder_hash: &str,
        kind: char,
        source_id: &str,
        chunk_idx: usize,
        revision: Option<&HLC>,
    ) -> Result<Option<EmbeddingData>> {
        let cf = cf_handle(&self.db, cf::EMBEDDINGS)?;

        if let Some(revision) = revision {
            let key = Self::embedding_key_v2(
                tenant_id,
                repo_id,
                branch,
                workspace_id,
                embedder_hash,
                kind,
                source_id,
                chunk_idx,
                revision,
            );
            return match self.db.get_cf(cf, &key).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to read embedding: {}", e))
            })? {
                Some(bytes) => Ok(Some(Self::deserialize(&bytes)?)),
                None => Ok(None),
            };
        }

        // Latest revision: revisions sort descending, so the first row wins.
        let prefix = Self::chunk_prefix_v2(
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            embedder_hash,
            kind,
            source_id,
            chunk_idx,
        );
        // Exactly one row is of interest, so take the first and stop; iterating
        // would read every later key in the CF for nothing.
        match self.scan_from(cf, &prefix).next() {
            Some(result) => {
                let (key, value) = result.map_err(|e| {
                    raisin_error::Error::storage(format!("Failed to iterate embeddings: {}", e))
                })?;
                if key.starts_with(&prefix) {
                    Ok(Some(Self::deserialize(&value)?))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    /// Every chunk row stored for one source under one embedder, as
    /// `(chunk_index, revision)`, at EVERY revision.
    ///
    /// The revision matters to the caller and is the whole reason this returns
    /// rows rather than a count. `cf::EMBEDDINGS` keys carry the revision, so a
    /// superseded chunk of an OLD revision is history and is correctly retained,
    /// while a chunk of the CURRENT revision that the current chunking no longer
    /// produces is an orphan that must go. Only the caller holding the revision
    /// it is writing can tell those two apart.
    #[allow(clippy::too_many_arguments)]
    pub fn list_source_chunks(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        embedder_hash: &str,
        kind: char,
        source_id: &str,
    ) -> Result<Vec<(usize, HLC)>> {
        let cf = cf_handle(&self.db, cf::EMBEDDINGS)?;
        let prefix = Self::source_prefix_v2(
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            embedder_hash,
            kind,
            source_id,
        );

        let mut rows = Vec::new();
        for result in self.scan_from(cf, &prefix) {
            let (key, _) = result.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate embeddings: {}", e))
            })?;
            if !key.starts_with(&prefix) {
                break;
            }
            let Some((_, _, _, chunk_idx, _)) = Self::parse_key(&key) else {
                continue;
            };
            if key.len() < 16 {
                continue;
            }
            let revision = HLC::decode_descending(&key[key.len() - 16..]).map_err(|e| {
                raisin_error::Error::storage(format!("Invalid HLC encoding: {}", e))
            })?;
            rows.push((chunk_idx, revision));
        }

        Ok(rows)
    }

    /// Delete the chunk rows of `source_id` AT `revision` whose index is
    /// `first_orphan` or higher. Returns the indexes actually deleted.
    ///
    /// This is the orphan sweep, and its narrowness is the point. Re-chunking a
    /// document into FEWER pieces leaves the surplus high-index chunks behind:
    /// nothing rewrites them, they keep matching queries, and anything that
    /// rebuilds the index reads them back out of RocksDB — so removing them from
    /// the index alone is not a fix. It deletes ONLY at the revision being
    /// written; an older revision's chunks are that revision's correct content
    /// and stay.
    #[allow(clippy::too_many_arguments)]
    pub fn prune_chunks_from(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        embedder_hash: &str,
        kind: char,
        source_id: &str,
        first_orphan: usize,
        revision: &HLC,
    ) -> Result<Vec<usize>> {
        let cf = cf_handle(&self.db, cf::EMBEDDINGS)?;

        let mut batch = WriteBatch::default();
        let mut pruned = Vec::new();

        for (chunk_idx, row_revision) in self.list_source_chunks(
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            embedder_hash,
            kind,
            source_id,
        )? {
            if chunk_idx < first_orphan || row_revision != *revision {
                continue;
            }
            let key = Self::embedding_key_v2(
                tenant_id,
                repo_id,
                branch,
                workspace_id,
                embedder_hash,
                kind,
                source_id,
                chunk_idx,
                revision,
            );
            batch.delete_cf(cf, &key);
            pruned.push(chunk_idx);
        }

        if !batch.is_empty() {
            self.db.write(batch).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to prune orphan chunks: {}", e))
            })?;
        }

        pruned.sort_unstable();
        Ok(pruned)
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
        let mut iter = self.scan_from(cf, &prefix);

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
        let iter = self.scan_from(cf, &ws_prefix);

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
        let iter = self.scan_from(cf, &legacy_prefix);
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
        let iter = self.scan_from(cf, &ws_prefix);
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
        let iter = self.scan_from(cf, &prefix);

        // Source ids in first-seen order, plus where each one sits in `results`
        // so a later revision can update it in place.
        let mut results: Vec<(String, HLC)> = Vec::new();
        let mut position: HashMap<String, usize> = HashMap::new();

        for result in iter {
            let (key, _) = result.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate embeddings: {}", e))
            })?;

            // Verify key matches prefix
            if !key.starts_with(&prefix) {
                break;
            }

            // Parse through `parse_key` — the ONE parser, shared with
            // `get_embedding`.
            //
            // This function used to read the source id out of `parts[4]` with
            // its own hand-rolled split. That is the LEGACY layout's node_id
            // slot; in the v2 layout part 4 is the EMBEDDER HASH, so every
            // caller got a list of embedder hashes rather than node ids, and
            // the dedup below then collapsed a whole workspace to one row per
            // embedder. `list_embeddings` is what an HNSW rebuild reads to know
            // WHICH nodes to re-index, so the rebuilt index pointed at ids that
            // resolve to no node at all.
            let Some((_, _, source_id, _, _)) = Self::parse_key(&key) else {
                continue;
            };

            if key.len() < 16 {
                continue;
            }
            let revision = HLC::decode_descending(&key[key.len() - 16..]).map_err(|e| {
                raisin_error::Error::storage(format!("Invalid HLC encoding: {}", e))
            })?;

            // One row per source, carrying its NEWEST revision. Revisions of a
            // single source are adjacent and descending, but a source reappears
            // once per embedder and once per chunk — non-adjacently — so
            // "differs from the previous key" is not a dedup.
            match position.get(&source_id) {
                Some(&index) => {
                    if revision > results[index].1 {
                        results[index].1 = revision;
                    }
                }
                None => {
                    position.insert(source_id.clone(), results.len());
                    results.push((source_id, revision));
                }
            }
        }

        Ok(results)
    }

    fn list_index_entries(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
    ) -> Result<Vec<StoredIndexEntry>> {
        let cf = cf_handle(&self.db, cf::EMBEDDINGS)?;
        let prefix = Self::workspace_prefix(tenant_id, repo_id, branch, workspace_id);

        // Keyed by the full address, so a chunk of a re-embedded document is not
        // confused with a different chunk of the same document. Revisions of one
        // chunk sort adjacent and descending, but chunks and embedders interleave
        // — "differs from the previous key" is not a dedup here any more than it
        // is in `list_embeddings`.
        let mut position: HashMap<(String, char, String, usize), usize> = HashMap::new();
        let mut results: Vec<StoredIndexEntry> = Vec::new();

        for result in self.scan_from(cf, &prefix) {
            let (key, _) = result.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate embeddings: {}", e))
            })?;
            if !key.starts_with(&prefix) {
                break;
            }
            let Some((embedder_hash, kind, source_id, chunk_index, _legacy)) =
                Self::parse_key(&key)
            else {
                continue;
            };
            if key.len() < 16 {
                continue;
            }
            let revision = HLC::decode_descending(&key[key.len() - 16..]).map_err(|e| {
                raisin_error::Error::storage(format!("Invalid HLC encoding: {}", e))
            })?;

            let address = (embedder_hash.clone(), kind, source_id.clone(), chunk_index);
            match position.get(&address) {
                Some(&index) => {
                    if revision > results[index].revision {
                        results[index].revision = revision;
                    }
                }
                None => {
                    position.insert(address, results.len());
                    results.push(StoredIndexEntry {
                        embedder_hash,
                        kind,
                        source_id,
                        chunk_index,
                        revision,
                    });
                }
            }
        }

        Ok(results)
    }

    fn list_workspaces(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<Vec<String>> {
        let cf = cf_handle(&self.db, cf::EMBEDDINGS)?;
        let prefix = Self::branch_prefix(tenant_id, repo_id, branch);
        let iter = self.scan_from(cf, &prefix);

        // BTreeSet so the result is deterministic (byte-sorted) regardless of
        // iteration order, and so a workspace with thousands of embeddings is
        // still reported once.
        let mut seen = std::collections::BTreeSet::new();

        for result in iter {
            let (key, _) = result.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate embeddings: {}", e))
            })?;

            if !key.starts_with(&prefix) {
                break;
            }

            // The workspace is the 4th null-delimited segment. Take it from the
            // bytes AFTER the branch prefix, up to the next null — a workspace
            // id cannot itself contain a null byte.
            let rest = &key[prefix.len()..];
            let Some(end) = rest.iter().position(|b| *b == 0) else {
                continue;
            };
            let Ok(workspace) = std::str::from_utf8(&rest[..end]) else {
                continue;
            };
            if !workspace.is_empty() && !seen.contains(workspace) {
                seen.insert(workspace.to_string());
            }
        }

        Ok(seen.into_iter().collect())
    }

    /// Delegates to the inherent [`Self::get_chunk`], which already builds the
    /// exact v2 key. It is on the TRAIT now because the caller that needs it —
    /// `VECTOR_OF(...)` in the SQL search surface — holds an
    /// `Arc<dyn EmbeddingStorage>` and cannot name the concrete type. The
    /// alternative was a second key builder on the SQL side, i.e. a second
    /// definition of the `cf::EMBEDDINGS` layout.
    fn get_source_chunk(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        embedder_hash: &str,
        kind: char,
        source_id: &str,
        chunk_idx: usize,
        revision: Option<&HLC>,
    ) -> Result<Option<EmbeddingData>> {
        self.get_chunk(
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            embedder_hash,
            kind,
            source_id,
            chunk_idx,
            revision,
        )
    }

    /// Delegates to the inherent [`Self::list_source_chunks`], collapsing its
    /// `(chunk_idx, revision)` rows to the distinct chunk indexes.
    ///
    /// `list_source_chunks` returns EVERY revision of every chunk, because its
    /// caller (the orphan sweep) has to tell a superseded old-revision chunk
    /// from an orphan of the current one. This caller is asking a different
    /// question — "how many vectors does this source have?" — and a document
    /// re-embedded three times must answer 1, not 3.
    fn list_source_chunk_indexes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        embedder_hash: &str,
        kind: char,
        source_id: &str,
    ) -> Result<Vec<usize>> {
        let rows = self.list_source_chunks(
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            embedder_hash,
            kind,
            source_id,
        )?;
        let distinct: std::collections::BTreeSet<usize> =
            rows.into_iter().map(|(idx, _)| idx).collect();
        Ok(distinct.into_iter().collect())
    }
}

/// THE test that stops the storage key and the index file name from drifting.
///
/// `EmbeddingPartition::to_index_token()` (in `raisin-ai`) is the only place a
/// partition is rendered, and `raisin-hnsw` takes the result as an opaque
/// `PartitionId` because it cannot see `EmbedderId` / `EmbeddingKind`. That
/// makes the two derivations *look* independent, which is exactly the shape
/// this codebase keeps losing things to. So: build a real key with the real
/// writer, cut segments 5 and 6 out of the bytes, and assert they concatenate to
/// the token byte for byte.
///
/// If someone changes the hash, the kind char, the segment order, or the token
/// format, this fails.
#[cfg(test)]
mod partition_token_matches_storage_key {
    use super::*;
    use raisin_ai::config::{EmbedderId, EmbeddingKind, EmbeddingPartition};

    /// Segments 5 and 6 (1-based) of a `cf::EMBEDDINGS` v2 key.
    fn embedder_and_kind_segments(key: &[u8]) -> (String, String) {
        let segments: Vec<&[u8]> = key.split(|b| *b == 0).collect();
        (
            String::from_utf8(segments[4].to_vec()).unwrap(),
            String::from_utf8(segments[5].to_vec()).unwrap(),
        )
    }

    fn assert_token_is_segments_5_and_6(embedder: EmbedderId, kind: EmbeddingKind) {
        let partition = EmbeddingPartition::new(embedder.clone(), kind);
        let key = RocksDBEmbeddingStorage::embedding_key_v2(
            "tenant",
            "repo",
            "main",
            "ws",
            &embedder.to_key_hash(),
            kind.to_key_char(),
            "node1",
            0,
            &HLC::new(1, 0),
        );

        let (hash_segment, kind_segment) = embedder_and_kind_segments(&key);
        assert_eq!(
            format!("{hash_segment}{kind_segment}"),
            partition.to_index_token(),
            "the HNSW partition token must be segments 5 and 6 of the cf::EMBEDDINGS key, \
             byte for byte — otherwise a vector is stored under one identity and indexed \
             under another, and nothing ever reports it"
        );
    }

    #[test]
    fn for_a_text_partition() {
        assert_token_is_segments_5_and_6(
            EmbedderId::new("ollama", "bge-m3", 1024),
            EmbeddingKind::Text,
        );
    }

    #[test]
    fn for_an_image_partition() {
        assert_token_is_segments_5_and_6(
            EmbedderId::new("ollama", "siglip-base", 768),
            EmbeddingKind::Image,
        );
    }

    #[test]
    fn two_models_of_the_same_width_get_different_tokens() {
        // The one case no width check can ever catch: same dimensions, unrelated
        // regions of R^n, every distance finite, nothing logs.
        let a = EmbeddingPartition::text(EmbedderId::new("ollama", "bge-m3", 1024));
        let b = EmbeddingPartition::text(EmbedderId::new("ollama", "mxbai-embed-large", 1024));
        assert_eq!(a.embedder.dimensions, b.embedder.dimensions);
        assert_ne!(a.to_index_token(), b.to_index_token());
    }

    #[test]
    fn text_and_image_of_one_model_are_different_partitions() {
        let e = EmbedderId::new("ollama", "siglip-base", 768);
        assert_ne!(
            EmbeddingPartition::text(e.clone()).to_index_token(),
            EmbeddingPartition::image(e).to_index_token()
        );
    }

    #[test]
    fn the_token_is_a_usable_file_stem() {
        let token =
            EmbeddingPartition::text(EmbedderId::new("ollama", "bge-m3", 1024)).to_index_token();
        assert!(
            raisin_hnsw::PartitionId::parse(&token).is_some(),
            "token {token:?} must survive PartitionId's file-stem validation"
        );
    }
}
