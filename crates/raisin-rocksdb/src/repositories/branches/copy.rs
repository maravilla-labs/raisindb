//! Branch index copying operations
//!
//! Provides functionality for physically copying revision-aware indexes
//! from one branch to another, used during branch creation.

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;
use std::sync::Arc;

use super::BranchRepositoryImpl;

/// Entries buffered per RocksDB write. Batching turns one WAL append per copied
/// entry into one per batch, which is the bulk of a fork's cost on a populated
/// workspace.
const COPY_BATCH_ENTRIES: usize = 1000;

impl BranchRepositoryImpl {
    /// Physically copy all revision-aware indexes from source branch to target branch
    /// Keeps the same revisions, only changes the branch name in the key.
    ///
    /// The work is a long, fully synchronous RocksDB scan-and-write over twelve
    /// column families, so it runs on a BLOCKING thread. Run directly on a Tokio
    /// worker it parks that worker for the whole fork, and on a populated
    /// workspace that starves unrelated requests on the same runtime — including
    /// the cheap `get_branch` point read the fork's own caller does next, which
    /// is how a fork could make an unrelated `branches().get("main")` hang until
    /// the client's 30s timeout.
    pub(crate) async fn copy_branch_indexes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        max_revision: &HLC,
    ) -> Result<()> {
        let db = Arc::clone(&self.db);
        let (tenant_id, repo_id) = (tenant_id.to_string(), repo_id.to_string());
        let (source_branch, target_branch) = (source_branch.to_string(), target_branch.to_string());
        let max_revision = *max_revision;
        tokio::task::spawn_blocking(move || {
            Self::copy_branch_indexes_blocking(
                &db,
                &tenant_id,
                &repo_id,
                &source_branch,
                &target_branch,
                &max_revision,
            )
        })
        .await
        .map_err(|e| raisin_error::Error::storage(format!("branch index copy panicked: {e}")))?
    }

    fn copy_branch_indexes_blocking(
        db: &DB,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        max_revision: &HLC,
    ) -> Result<()> {
        // Copy indexes from these column families:
        // 1. NODES - actual node data
        // 2. PATH_INDEX - path-based lookups
        // 3. PROPERTY_INDEX - property-based lookups
        // 4. REFERENCE_INDEX - reference relationships
        // 5. ORDERED_CHILDREN - child ordering
        // 6. RELATION_INDEX - graph relations
        // 7. NODE_TYPES - schema definitions and version indexes

        let cfs_to_copy = vec![
            (cf::NODES, "nodes"),
            (cf::PATH_INDEX, "path_index"),
            (cf::PROPERTY_INDEX, "property_index"),
            (cf::REFERENCE_INDEX, "reference_index"),
            (cf::RELATION_INDEX, "relation_index"),
            (cf::ORDERED_CHILDREN, "ordered_children"),
            (cf::NODE_TYPES, "node_types"),
            (cf::ARCHETYPES, "archetypes"),
            (cf::ELEMENT_TYPES, "element_types"),
            (cf::NODE_PATH, "node_path"),
            // Translation CFs - translations are part of the node and must be copied
            (cf::TRANSLATION_DATA, "translation_data"),
            (cf::BLOCK_TRANSLATIONS, "block_translations"),
        ];

        for (cf_name, display_name) in cfs_to_copy {
            let copied = Self::copy_cf_entries(
                db,
                tenant_id,
                repo_id,
                source_branch,
                target_branch,
                max_revision,
                cf_name,
            )?;

            tracing::info!(
                "Copied {} entries from {} CF (branch: {} -> {})",
                copied,
                display_name,
                source_branch,
                target_branch
            );
        }

        Ok(())
    }

    /// Copy entries from a specific column family, preserving revisions.
    /// Synchronous by design — see `copy_branch_indexes`.
    fn copy_cf_entries(
        db: &DB,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        max_revision: &HLC,
        cf_name: &str,
    ) -> Result<usize> {
        let cf = cf_handle(db, cf_name)?;

        // Build prefix for source branch: {tenant}\0{repo}\0{source_branch}\0
        let source_prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(source_branch)
            .build_prefix();

        let source_prefix_clone = source_prefix.clone();

        // For ORDERED_CHILDREN, use iterator_cf with seek instead of prefix_iterator_cf
        // because the CF has a custom prefix extractor configured
        use rocksdb::IteratorMode;
        let iter = if cf_name == cf::ORDERED_CHILDREN {
            db.iterator_cf(
                &cf,
                IteratorMode::From(&source_prefix_clone, rocksdb::Direction::Forward),
            )
        } else {
            db.prefix_iterator_cf(&cf, source_prefix)
        };

        let mut copied_count = 0;
        let mut skipped_unparseable = 0usize;
        let mut batch = rocksdb::WriteBatch::default();
        let mut pending = 0usize;

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&source_prefix_clone) {
                break;
            }

            // Parse the key to extract the revision
            // Key formats:
            // NODES: {tenant}\0{repo}\0{branch}\0{workspace}\0nodes\0{node_id}\0{~revision}
            // PATH_INDEX: {tenant}\0{repo}\0{branch}\0{workspace}\0path\0{path}\0{~revision}
            // PROPERTY_INDEX: {tenant}\0{repo}\0{branch}\0{workspace}\0prop{_pub}\0{property_name}\0{value_hash}\0{~revision}\0{node_id}
            // REFERENCE_INDEX (forward): {tenant}\0{repo}\0{branch}\0{workspace}\0ref{_pub}\0{node_id}\0{property_path}\0{~revision}
            // REFERENCE_INDEX (reverse): {tenant}\0{repo}\0{branch}\0{workspace}\0ref_rev{_pub}\0{target_workspace}\0{target_path}\0{source_node_id}\0{property_path}\0{~revision}
            // RELATION_INDEX (forward): {tenant}\0{repo}\0{branch}\0{workspace}\0rel\0{source_node_id}\0{relation_type}\0{~revision}\0{target_node_id}
            // RELATION_INDEX (reverse): {tenant}\0{repo}\0{branch}\0{workspace}\0rel_rev\0{target_node_id}\0{relation_type}\0{~revision}\0{source_node_id}
            // ORDERED_CHILDREN: {tenant}\0{repo}\0{branch}\0{workspace}\0ordered\0{parent_id}\0{order_label}\0{~revision}\0{child_id}

            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();

            // Find the revision bytes (they're encoded as descending u64)
            // The revision index varies by CF and key structure.
            //
            // ORDERED_CHILDREN is special: its key embeds a 16-byte ~HLC that can
            // itself contain null bytes, so naive split-based indexing mis-locates
            // the revision and silently DROPS the entry (revision_opt == None). Parse
            // it directly from the raw key instead, walking null boundaries like the
            // query path does.
            let revision_opt = if cf_name == cf::ORDERED_CHILDREN {
                Self::extract_ordered_children_revision(&key)
            } else {
                Self::extract_revision_from_key_parts(&parts, cf_name, &value, &key)?
            }
            // Last resort for the CFs whose revision sits in the MIDDLE of the
            // key (a node id follows it), where the null-split arms above still
            // mis-index when the ~HLC contains a null. Strictly additive: it only
            // runs where the entry would otherwise have been dropped outright.
            .or_else(|| Self::positional_revision_fallback(cf_name, &key));

            // An entry whose revision we cannot read is DROPPED from the fork.
            // That is data loss disguised as a successful copy — the forked
            // branch simply doesn't contain that node, which downstream looks
            // like "the page doesn't exist" rather than "the copy was lossy".
            // Count them so a lossy fork is at least loud.
            if revision_opt.is_none() {
                skipped_unparseable += 1;
            }

            // Only copy if revision <= max_revision
            if let Some(revision) = revision_opt {
                if &revision <= max_revision {
                    // Create new key with target branch instead of source branch
                    // Replace the branch component (index 2) with target_branch
                    let new_key = Self::build_key_with_branch(&parts, target_branch);

                    // Write to target branch with same value (preserving revision)
                    batch.put_cf(&cf, new_key, &*value);
                    pending += 1;
                    copied_count += 1;

                    if pending >= COPY_BATCH_ENTRIES {
                        db.write(std::mem::take(&mut batch))
                            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
                        pending = 0;
                    }
                }
            }
        }

        if pending > 0 {
            db.write(batch)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        }

        if skipped_unparseable > 0 {
            tracing::warn!(
                cf = cf_name,
                skipped = skipped_unparseable,
                copied = copied_count,
                source_branch,
                target_branch,
                "Branch fork DROPPED entries whose revision could not be parsed — \
                 the forked branch is missing this content"
            );
        }

        Ok(copied_count)
    }

    /// Recover a revision from a key whose null-split parsing failed.
    ///
    /// Both shapes are located WITHOUT splitting, so a null byte inside the
    /// 16-byte `~HLC` cannot shift them:
    ///  - PROPERTY_INDEX / RELATION_INDEX / ORDERED_CHILDREN end with
    ///    `…\0{~revision}\0{node_id}`. The trailing id is a uuid and contains no
    ///    nulls, so the LAST null in the key is exactly the separator after the
    ///    revision — take the 16 bytes before it.
    ///  - Everything else ends with the revision, so take the last 16 bytes.
    ///
    /// Returns None rather than a guess when the key is too short to hold one.
    fn positional_revision_fallback(cf_name: &str, key: &[u8]) -> Option<HLC> {
        let id_follows_revision = cf_name == cf::PROPERTY_INDEX
            || cf_name == cf::RELATION_INDEX
            || cf_name == cf::ORDERED_CHILDREN;
        if id_follows_revision {
            let last_null = key.iter().rposition(|&b| b == 0)?;
            let start = last_null.checked_sub(16)?;
            return keys::decode_descending_revision(&key[start..last_null]).ok();
        }
        keys::extract_revision_from_key(key).ok()
    }

    /// Extract revision from key parts based on column family structure.
    ///
    /// `key` is the raw, unsplit key. It is needed because a `~HLC` is 16 raw
    /// bytes that MAY CONTAIN NULLS, so `key.split(0)` can shred it into several
    /// "parts" and shift every index after it — see `extract_trailing_revision`.
    fn extract_revision_from_key_parts(
        parts: &[&[u8]],
        cf_name: &str,
        value: &[u8],
        key: &[u8],
    ) -> Result<Option<HLC>> {
        let revision_opt = if cf_name == cf::NODE_TYPES {
            match parts.get(3).copied() {
                Some(segment) if segment == b"nodetypes" => {
                    if let Some(rev_part) = parts.get(5) {
                        if rev_part.len() == 16 {
                            keys::decode_descending_revision(rev_part).ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                Some(segment) if segment == b"nodetype_versions" => {
                    if value.len() == 16 {
                        HLC::decode_descending(value).ok()
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else if cf_name == cf::ARCHETYPES {
            // Schema key: {tenant}\0{repo}\0{branch}\0archetypes\0{name}\0{~revision}
            //   -> revision at index 5
            // Version index: {tenant}\0{repo}\0{branch}\0archetype_versions\0{name}\0{version}
            //   -> revision stored in the value (descending HLC)
            match parts.get(3).copied() {
                Some(segment) if segment == b"archetypes" => match parts.get(5) {
                    Some(rev_part) if rev_part.len() == 16 => {
                        keys::decode_descending_revision(rev_part).ok()
                    }
                    _ => None,
                },
                Some(segment) if segment == b"archetype_versions" => {
                    if value.len() == 16 {
                        HLC::decode_descending(value).ok()
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else if cf_name == cf::ELEMENT_TYPES {
            // Schema key: {tenant}\0{repo}\0{branch}\0element_types\0{name}\0{~revision}
            //   -> revision at index 5
            // Version index: {tenant}\0{repo}\0{branch}\0element_type_versions\0{name}\0{version}
            //   -> revision stored in the value (descending HLC)
            match parts.get(3).copied() {
                Some(segment) if segment == b"element_types" => match parts.get(5) {
                    Some(rev_part) if rev_part.len() == 16 => {
                        keys::decode_descending_revision(rev_part).ok()
                    }
                    _ => None,
                },
                Some(segment) if segment == b"element_type_versions" => {
                    if value.len() == 16 {
                        HLC::decode_descending(value).ok()
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else if cf_name == cf::RELATION_INDEX && parts.len() >= 9 {
            // RELATION_INDEX: revision is at index 7 (both forward and reverse)
            if parts[7].len() == 16 {
                keys::decode_descending_revision(parts[7]).ok()
            } else {
                None
            }
        } else if cf_name == cf::ORDERED_CHILDREN && parts.len() >= 9 {
            // ORDERED_CHILDREN: revision is at index 7
            if parts[7].len() == 16 {
                keys::decode_descending_revision(parts[7]).ok()
            } else {
                None
            }
        } else if cf_name == cf::PROPERTY_INDEX && parts.len() >= 9 {
            // PROPERTY_INDEX: revision is at index 7
            if parts[7].len() == 16 {
                keys::decode_descending_revision(parts[7]).ok()
            } else {
                None
            }
        } else if cf_name == cf::REFERENCE_INDEX && parts.len() >= 10 {
            // REFERENCE_INDEX (reverse): Check if this is a reverse key (ref_rev or ref_rev_pub)
            // by checking if index 9 is a 16-byte HLC revision
            if parts[9].len() == 16 {
                keys::decode_descending_revision(parts[9]).ok()
            } else if parts.len() >= 8 && parts[7].len() == 16 {
                // REFERENCE_INDEX (forward): revision is at index 7
                keys::decode_descending_revision(parts[7]).ok()
            } else {
                None
            }
        } else if cf_name == cf::REFERENCE_INDEX && parts.len() >= 8 {
            // REFERENCE_INDEX (forward): revision is at index 7
            if parts[7].len() == 16 {
                keys::decode_descending_revision(parts[7]).ok()
            } else {
                None
            }
        } else if cf_name == cf::TRANSLATION_DATA && parts.len() >= 8 {
            // TRANSLATION_DATA: {tenant}\0{repo}\0{branch}\0{ws}\0translations\0{node_id}\0{locale}\0{revision}
            // revision is at index 7
            if parts[7].len() == 16 {
                keys::decode_descending_revision(parts[7]).ok()
            } else {
                None
            }
        } else if cf_name == cf::BLOCK_TRANSLATIONS && parts.len() >= 9 {
            // BLOCK_TRANSLATIONS: {tenant}\0{repo}\0{branch}\0{ws}\0block_trans\0{node_id}\0{block_uuid}\0{locale}\0{revision}
            // revision is at index 8
            if parts[8].len() == 16 {
                keys::decode_descending_revision(parts[8]).ok()
            } else {
                None
            }
        } else if parts.len() >= 7 {
            // NODES, PATH_INDEX, NODE_PATH (and node adjacency) all END with the
            // revision — `node_key_versioned`/`path_index_key_versioned`/
            // `node_path_key_versioned` all `push_revision(...)` last. So read it
            // from the TAIL of the raw key, not from `parts[6]`.
            //
            // This is the same trap already documented for ORDERED_CHILDREN, and
            // `keys::extract_revision_from_key` exists precisely for it: the
            // 16-byte ~HLC is a bitwise-NOT encoding that CAN contain null bytes,
            // so `split(0)` shreds the revision itself and `parts[6]` is a
            // fragment of the wrong length. The old code then returned None and
            // the entry was SILENTLY SKIPPED — so every fork randomly lost
            // whichever nodes happened to have a null byte in their revision
            // (~0.8% of them measured), and those pages were simply absent from
            // the branch, which reads downstream as "that page doesn't exist".
            keys::extract_revision_from_key(key).ok()
        } else {
            None
        };

        Ok(revision_opt)
    }

    /// Extract the revision from an ORDERED_CHILDREN key by walking null
    /// boundaries (the embedded ~HLC may contain null bytes, so `key.split(\0)`
    /// cannot be trusted here).
    ///
    /// Key: `{tenant}\0{repo}\0{branch}\0{workspace}\0ordered\0{parent_id}\0{order_label}\0{~HLC-16}\0{child_id}`
    fn extract_ordered_children_revision(key: &[u8]) -> Option<HLC> {
        // Walk to the 6th null byte (end of parent_id == start of order_label).
        let mut null_count = 0;
        let mut prefix_end = 0;
        for (i, &byte) in key.iter().enumerate() {
            if byte == 0 {
                null_count += 1;
                if null_count == 6 {
                    prefix_end = i + 1;
                    break;
                }
            }
        }
        if null_count < 6 {
            return None;
        }

        let after_prefix = &key[prefix_end..];
        // order_label runs until the next null; the 16-byte ~HLC follows it.
        let order_label_end = after_prefix.iter().position(|&b| b == 0)?;
        let hlc_start = order_label_end + 1;
        let hlc_end = hlc_start + 16;
        if after_prefix.len() < hlc_end {
            return None;
        }
        keys::decode_descending_revision(&after_prefix[hlc_start..hlc_end]).ok()
    }

    /// Build a new key with a different branch name
    fn build_key_with_branch(parts: &[&[u8]], target_branch: &str) -> Vec<u8> {
        let mut new_key_parts = parts.iter().map(|p| p.to_vec()).collect::<Vec<_>>();
        new_key_parts[2] = target_branch.as_bytes().to_vec();

        // Rebuild the key
        let mut new_key = Vec::new();
        for (i, part) in new_key_parts.iter().enumerate() {
            if i > 0 {
                new_key.push(0); // null byte separator
            }
            new_key.extend_from_slice(part);
        }

        new_key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_hlc::HLC;

    /// Find an HLC whose descending encoding contains a null byte. These are
    /// common — the encoding is a bitwise NOT, so any 0xFF byte in the source
    /// becomes 0x00 — which is why they cannot be located by splitting on nulls.
    fn hlc_with_null_in_encoding() -> HLC {
        for counter in 0..4096u64 {
            let hlc = HLC::new(0x0000_00FF_0000_0000, counter);
            if hlc.encode_descending().contains(&0) {
                return hlc;
            }
        }
        panic!("no HLC with a null byte in its encoding found");
    }

    /// A fork must copy EVERY node, including those whose revision encoding
    /// happens to contain a null byte.
    ///
    /// Regression: the NODES/PATH_INDEX arm read the revision from
    /// `key.split(0)[6]`. When the 16-byte ~HLC contained a null, the split cut
    /// the revision in half, that index held a short fragment, extraction
    /// returned None — and the copy loop skipped the entry WITHOUT error. Every
    /// forked branch silently lost ~0.8% of its nodes, so random pages 404'd in
    /// the SSR preview while the fork reported success.
    #[test]
    fn revision_is_read_from_keys_whose_hlc_contains_a_null_byte() {
        let hlc = hlc_with_null_in_encoding();
        let key = keys::node_key_versioned("t", "r", "main", "ws", "node-1", &hlc);

        assert!(
            key.split(|&b| b == 0).count() > 7,
            "this key must actually exercise the split hazard",
        );

        let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        let got =
            BranchRepositoryImpl::extract_revision_from_key_parts(&parts, cf::NODES, &[], &key)
                .expect("extraction must not error");

        assert_eq!(got, Some(hlc), "the entry would otherwise be dropped");
    }

    /// The ordinary case must keep working.
    #[test]
    fn revision_is_read_from_a_plain_node_key() {
        let hlc = HLC::new(12_345, 7);
        let key = keys::node_key_versioned("t", "r", "main", "ws", "node-1", &hlc);
        let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        let got =
            BranchRepositoryImpl::extract_revision_from_key_parts(&parts, cf::NODES, &[], &key)
                .unwrap();
        assert_eq!(got, Some(hlc));
    }

    /// Rebuilding the key must preserve a null-containing revision byte-for-byte,
    /// or the copied entry lands under a corrupted key.
    #[test]
    fn rebuilt_key_only_swaps_the_branch_segment() {
        let hlc = hlc_with_null_in_encoding();
        let key = keys::node_key_versioned("t", "r", "main", "ws", "node-1", &hlc);
        let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        let rebuilt = BranchRepositoryImpl::build_key_with_branch(&parts, "edit-x");

        let expected = keys::node_key_versioned("t", "r", "edit-x", "ws", "node-1", &hlc);
        assert_eq!(rebuilt, expected);
        assert_eq!(
            keys::extract_revision_from_key(&rebuilt).unwrap(),
            hlc,
            "the revision must survive the rebuild",
        );
    }
}
