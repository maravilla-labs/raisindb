//! Branch index copying operations
//!
//! Provides functionality for physically copying revision-aware indexes
//! from one branch to another, used during branch creation.

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;

use super::BranchRepositoryImpl;

impl BranchRepositoryImpl {
    /// Physically copy all revision-aware indexes from source branch to target branch
    /// Keeps the same revisions, only changes the branch name in the key
    pub(crate) async fn copy_branch_indexes(
        &self,
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
            let copied = self
                .copy_cf_entries(
                    tenant_id,
                    repo_id,
                    source_branch,
                    target_branch,
                    max_revision,
                    cf_name,
                )
                .await?;

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

    /// Copy entries from a specific column family, preserving revisions
    async fn copy_cf_entries(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        max_revision: &HLC,
        cf_name: &str,
    ) -> Result<usize> {
        let cf = cf_handle(&self.db, cf_name)?;

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
            self.db.iterator_cf(
                &cf,
                IteratorMode::From(&source_prefix_clone, rocksdb::Direction::Forward),
            )
        } else {
            self.db.prefix_iterator_cf(&cf, source_prefix)
        };

        let mut copied_count = 0;

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
            // The revision index varies by CF and key structure
            let revision_opt = self.extract_revision_from_key_parts(&parts, cf_name, &value)?;

            // Only copy if revision <= max_revision
            if let Some(revision) = revision_opt {
                if &revision <= max_revision {
                    // Create new key with target branch instead of source branch
                    // Replace the branch component (index 2) with target_branch
                    let new_key = self.build_key_with_branch(&parts, target_branch);

                    // Write to target branch with same value (preserving revision)
                    self.db
                        .put_cf(&cf, new_key, &*value)
                        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

                    copied_count += 1;
                }
            }
        }

        Ok(copied_count)
    }

    /// Extract revision from key parts based on column family structure
    fn extract_revision_from_key_parts(
        &self,
        parts: &[&[u8]],
        cf_name: &str,
        value: &[u8],
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
            // NODES and PATH_INDEX: revision is at index 6
            // (0=tenant, 1=repo, 2=branch, 3=workspace, 4=type, 5=id/path, 6=revision)
            if parts[6].len() == 16 {
                keys::decode_descending_revision(parts[6]).ok()
            } else {
                None
            }
        } else {
            None
        };

        Ok(revision_opt)
    }

    /// Build a new key with a different branch name
    fn build_key_with_branch(&self, parts: &[&[u8]], target_branch: &str) -> Vec<u8> {
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
