//! `get_embedding`'s v2 fallback: one source's row, found with bounded seeks.
//!
//! `get_embedding` takes no embedder, so for the v2 layout
//! (`{ws}\0{embedder_hash}\0{kind}\0{source}\0{chunk}\0{~rev}`) it cannot
//! address a key directly. It used to scan the WHOLE workspace prefix, every
//! row of every source, until it met one whose source id matched — per node,
//! so a `SELECT` over a workspace was quadratic in its vector count.
//!
//! The answer is unchanged: the first row in KEY ORDER whose source id
//! matches. Key order is partition `(embedder_hash, kind)` first, then source,
//! so the scan visits partitions in order and, in each one, seeks straight to
//! `{partition}{source}\0`. It costs one seek per partition present in the
//! workspace — a handful — instead of one step per stored row.
//!
//! With a revision, the row is the NEWEST at or before it. The old fallback
//! demanded an exact revision match, but an embedding is written at the
//! revision of the node version it was computed from, and a node read at any
//! later revision passes that later revision: the vector was simply never
//! found (the NULL `SELECT embedding` column).

use super::RocksDBEmbeddingStorage;
use raisin_embeddings::EmbeddingData;
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{DBRawIteratorWithThreadMode, ReadOptions, DB};

impl RocksDBEmbeddingStorage {
    /// The first v2 row of `source_id` in key order, at the newest revision
    /// `<= max_revision` (the newest at all when `None`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn find_v2_source_row(
        &self,
        cf: &impl rocksdb::AsColumnFamilyRef,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        source_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<EmbeddingData>> {
        let ws_prefix = Self::workspace_prefix(tenant_id, repo_id, branch, workspace_id);

        let mut opts = ReadOptions::default();
        opts.set_total_order_seek(true);
        if let Some(upper) = crate::prefix_successor(&ws_prefix) {
            opts.set_iterate_upper_bound(upper);
        }
        let mut iter = self.db.raw_iterator_cf_opt(cf, opts);

        let mut cursor = ws_prefix.clone();
        loop {
            iter.seek(&cursor);
            if !iter.valid() {
                break;
            }
            let Some(key) = iter.key() else {
                break;
            };
            if !key.starts_with(&ws_prefix) {
                break;
            }

            // Classify exactly as the old row-by-row scan did (`parse_key`),
            // but build prefixes from the raw key bytes. Owned, so the key's
            // borrow of the iterator ends before the iterator moves again.
            let is_legacy = Self::parse_key(key).map(|(_, _, _, _, legacy)| legacy);
            let mut segments = key[ws_prefix.len()..].splitn(3, |b| *b == 0);
            let first = segments.next().unwrap_or_default().to_vec();
            let second = segments.next().map(<[u8]>::to_vec);

            let skip_past = match (is_legacy, second) {
                // A v2 row: `{embedder_hash}\0{kind}\0` is its partition.
                // Look for the source in it, then skip the whole partition.
                (Some(false), Some(kind)) => {
                    let mut partition = ws_prefix.clone();
                    partition.extend_from_slice(&first);
                    partition.push(0);
                    partition.extend_from_slice(&kind);
                    partition.push(0);

                    let mut source_prefix = partition.clone();
                    source_prefix.extend_from_slice(source_id.as_bytes());
                    source_prefix.push(0);
                    let row = first_row_at_or_before(&mut iter, &source_prefix, max_revision)?;
                    if row.is_some() {
                        return Ok(row);
                    }
                    partition
                }
                // A legacy row (`{node_id}\0{~rev}`): `get_embedding` has
                // already answered from the legacy layout for this source, so
                // skip every revision of that node at once.
                (Some(true), _) => {
                    let mut node = ws_prefix.clone();
                    node.extend_from_slice(&first);
                    node.push(0);
                    node
                }
                // Unparseable: step past this one key.
                _ => {
                    let mut next = key.to_vec();
                    next.push(0);
                    cursor = next;
                    continue;
                }
            };

            match crate::prefix_successor(&skip_past) {
                Some(next) => cursor = next,
                None => break,
            }
        }

        iter.status().map_err(|e| {
            raisin_error::Error::storage(format!("Failed to iterate embeddings: {}", e))
        })?;
        Ok(None)
    }
}

/// Within one source's rows (`{source_prefix}{chunk}\0{~rev}`), the first
/// chunk that has a version `<= max_revision`, at that version.
///
/// Each chunk's versions run newest first, so a chunk whose newest version is
/// too new is left with one seek to `{chunk}\0{~max}`, which lands on its
/// newest acceptable version or on the next chunk.
fn first_row_at_or_before(
    iter: &mut DBRawIteratorWithThreadMode<'_, DB>,
    source_prefix: &[u8],
    max_revision: Option<&HLC>,
) -> Result<Option<EmbeddingData>> {
    let mut seek = source_prefix.to_vec();
    loop {
        iter.seek(&seek);
        if !iter.valid() {
            return Ok(None);
        }
        let (Some(key), Some(value)) = (iter.key(), iter.value()) else {
            return Ok(None);
        };
        if !key.starts_with(source_prefix) {
            return Ok(None);
        }
        // Too short to hold a chunk and a revision: step past it.
        if key.len() < source_prefix.len() + 16 {
            seek = key.to_vec();
            seek.push(0);
            continue;
        }

        let (chunk_part, revision_bytes) = key.split_at(key.len() - 16);
        let revision = HLC::decode_descending(revision_bytes)
            .map_err(|e| raisin_error::Error::storage(format!("Invalid HLC encoding: {}", e)))?;
        match max_revision {
            Some(max) if revision > *max => {
                // Strictly past `key`: `~max` sorts after `~revision`.
                seek = chunk_part.to_vec();
                seek.extend_from_slice(&max.encode_descending());
            }
            _ => return Ok(Some(RocksDBEmbeddingStorage::deserialize(value)?)),
        }
    }
}
