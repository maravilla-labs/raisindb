//! Branch index copying operations
//!
//! Provides functionality for physically copying revision-aware indexes
//! from one branch to another, used during branch creation.

use crate::{cf, cf_handle, keys, prefix_transform};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;
use std::sync::Arc;

use super::cf_registry::{cfs_to_copy, CopyPlan, RevisionLocator};
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
        // WHICH column families get copied is NOT decided here. It comes from
        // `super::cf_registry::BRANCH_CF_REGISTRY`, the exhaustive table that
        // classifies every CF the database defines. A hand-maintained list
        // lived here before and fell behind reality twice — silently dropping
        // ARCHETYPES/ELEMENT_TYPES, then the whole SPATIAL_INDEX — because a CF
        // that was never mentioned was simply never copied and nothing said so.
        for (cf_name, plan) in cfs_to_copy() {
            let copied = Self::copy_cf_entries(
                db,
                tenant_id,
                repo_id,
                source_branch,
                target_branch,
                max_revision,
                cf_name,
                &plan,
            )?;

            tracing::info!(
                "Copied {} entries from {} CF (branch: {} -> {})",
                copied,
                plan.display_name,
                source_branch,
                target_branch
            );
        }

        Ok(())
    }

    /// Copy entries from a specific column family, preserving revisions.
    /// Synchronous by design — see `copy_branch_indexes`.
    #[allow(clippy::too_many_arguments)]
    fn copy_cf_entries(
        db: &DB,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        max_revision: &HLC,
        cf_name: &str,
        plan: &CopyPlan,
    ) -> Result<usize> {
        let cf = cf_handle(db, cf_name)?;

        // Build prefix for source branch: {tenant}\0{repo}\0{source_branch}\0
        let source_prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(source_branch)
            .build_prefix();

        let source_prefix_clone = source_prefix.clone();

        // A CF with a custom prefix extractor (ORDERED_CHILDREN, SPATIAL_INDEX)
        // CANNOT be scanned with `prefix_iterator_cf` here: our prefix stops at
        // the branch, which is far shorter than the extractor's domain, so the
        // prefix bloom filter is consulted for a prefix it never produces and
        // the scan can come back empty. Seek with a plain iterator instead and
        // do the prefix comparison ourselves (the loop already does).
        //
        // Asking `prefix_transform` rather than naming CFs here means adding a
        // third extractor cannot silently break the fork.
        use rocksdb::IteratorMode;
        let iter = if prefix_transform::has_custom_prefix_extractor(cf_name) {
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

            // Locate the 16-byte descending HLC. Which strategy applies is the
            // registry's declaration, not a guess from the CF name.
            //
            // Every strategy locates it POSITIONALLY, never by indexing into
            // `parts`: a `~HLC` is a bitwise NOT and may contain null bytes, so
            // `split(0)` shreds it and shifts every index after it. That bug
            // silently dropped ~0.8% of nodes from every fork.
            let revision_opt = Self::locate_revision(plan.revision, &key, &value, &parts)?;

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

    /// Locate a key's revision using the strategy the registry declares.
    ///
    /// Returns `Ok(None)` when this key genuinely carries no readable revision;
    /// the caller counts and warns about those, because dropping one is data
    /// loss disguised as a successful fork.
    pub(super) fn locate_revision(
        locator: RevisionLocator,
        key: &[u8],
        value: &[u8],
        parts: &[&[u8]],
    ) -> Result<Option<HLC>> {
        Ok(match locator {
            RevisionLocator::Tail => keys::extract_revision_from_key(key).ok(),
            RevisionLocator::BeforeTrailingSegments(n) => {
                Self::revision_before_trailing_segments(key, n)
            }
            RevisionLocator::RelationIndex => {
                // `rel_global` keys carry FOUR trailing segments
                // (`…{~rev}\0{src_ws}\0{src_id}\0{tgt_ws}\0{tgt_id}`) where
                // `rel` / `rel_rev` carry one. Reading them as one lands 16
                // arbitrary bytes, which decode into a nonsense HLC that is
                // then compared against `max_revision` — so a global relation
                // was dropped from, or mis-filtered into, every fork.
                let trailing = if parts.get(3).copied() == Some(b"rel_global".as_slice()) {
                    4
                } else {
                    1
                };
                Self::revision_before_trailing_segments(key, trailing)
            }
            RevisionLocator::SpatialKey => {
                keys::parse_spatial_index_key(key).map(|parsed| parsed.revision)
            }
            RevisionLocator::SchemaOrVersionValue => Self::schema_revision(parts, value, key)?,
        })
    }

    /// Read the revision from a key that ends with `{~revision}` followed by `n`
    /// null-free segments (node ids, workspace names).
    ///
    /// Walks back `n` null bytes from the END of the key, then takes the 16
    /// bytes before that null. Nothing here splits the key, so a null byte
    /// INSIDE the `~HLC` cannot shift the result — which is the whole point.
    fn revision_before_trailing_segments(key: &[u8], n: usize) -> Option<HLC> {
        let mut boundary = key.len();
        for _ in 0..n {
            boundary = key[..boundary].iter().rposition(|&b| b == 0)?;
        }
        let start = boundary.checked_sub(16)?;
        keys::decode_descending_revision(&key[start..boundary]).ok()
    }

    /// Revision for the schema CFs (`NODE_TYPES`, `ARCHETYPES`, `ELEMENT_TYPES`).
    ///
    /// These have two shapes: the definition itself ends with the revision,
    /// while the `*_versions` lookup index stores a descending HLC in the VALUE.
    /// The discriminator is the literal tag at part 3, which is plain ASCII and
    /// precedes any `~HLC`, so indexing into `parts` is safe there.
    fn schema_revision(parts: &[&[u8]], value: &[u8], key: &[u8]) -> Result<Option<HLC>> {
        let cf_name = match parts.get(3).copied() {
            Some(b"nodetypes") | Some(b"nodetype_versions") => cf::NODE_TYPES,
            Some(b"archetypes") | Some(b"archetype_versions") => cf::ARCHETYPES,
            Some(b"element_types") | Some(b"element_type_versions") => cf::ELEMENT_TYPES,
            _ => return Ok(None),
        };
        Self::extract_revision_from_key_parts(parts, cf_name, value, key)
    }

    /// Extract the revision for one of the three SCHEMA column families.
    ///
    /// Two shapes per CF:
    ///  - the definition, `{tenant}\0{repo}\0{branch}\0{tag}\0{name}\0{~revision}`,
    ///    where the revision is the last segment; and
    ///  - the `*_versions` lookup, `…\0{tag}\0{name}\0{version}`, which carries no
    ///    revision in the key at all — it is a descending HLC in the VALUE.
    ///
    /// `parts` indexing is safe here only because the discriminating tag at
    /// index 3 is plain ASCII and precedes any `~HLC`.
    fn extract_revision_from_key_parts(
        parts: &[&[u8]],
        cf_name: &str,
        value: &[u8],
        key: &[u8],
    ) -> Result<Option<HLC>> {
        let (definition_tag, versions_tag) = match cf_name {
            cf::NODE_TYPES => (b"nodetypes".as_slice(), b"nodetype_versions".as_slice()),
            cf::ARCHETYPES => (b"archetypes".as_slice(), b"archetype_versions".as_slice()),
            cf::ELEMENT_TYPES => (
                b"element_types".as_slice(),
                b"element_type_versions".as_slice(),
            ),
            _ => return Ok(None),
        };

        let revision_opt = match parts.get(3).copied() {
            // Read from the TAIL of the raw key rather than `parts[5]`: a
            // `~HLC` may contain null bytes, and `split(0)` then leaves a
            // short fragment at index 5 and the entry gets DROPPED.
            Some(tag) if tag == definition_tag => keys::extract_revision_from_key(key).ok(),
            Some(tag) if tag == versions_tag => {
                if value.len() == 16 {
                    HLC::decode_descending(value).ok()
                } else {
                    None
                }
            }
            _ => None,
        };

        Ok(revision_opt)
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
#[path = "copy_tests.rs"]
mod tests;
