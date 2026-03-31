//! Query operations for child ordering
//!
//! This module provides efficient query functions for retrieving order labels
//! and child lists without loading full node objects.

use super::super::helpers::is_tombstone;
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use std::collections::HashSet;

impl NodeRepositoryImpl {
    /// Get the current HEAD order label for a specific child (EFFICIENT - O(1))
    ///
    /// This method scans only the parent's ordered index and extracts the
    /// order_label for the specified child. No full child list loading.
    ///
    /// Returns None if the child is not found in the ordered index.
    pub(crate) fn get_order_label_for_child(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        child_id: &str,
    ) -> Result<Option<String>> {
        let prefix =
            keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_ordered, prefix);

        // Track seen (order_label, child_id) pairs to handle MVCC properly
        // With descending HLC, newer entries come first - we want the most recent non-tombstone
        let mut seen_labels: HashSet<(String, String)> = HashSet::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            // IMPORTANT: Cannot use null-byte splitting for the entire key because
            // HLC's descending encoding can contain null bytes (especially counter=0).
            //
            // Key structure: {tenant}\0{repo}\0{branch}\0{workspace}\0ORDERED_CHILDREN\0{parent_id}\0{order_label}\0{~HLC-16bytes}\0{child_id}
            //
            // Strategy: Find the 6th null byte to locate the start of order_label

            // Find the 6th null byte (end of prefix)
            let mut null_count = 0;
            let mut prefix_end = 0;
            for (i, &byte) in key.iter().enumerate() {
                if byte == 0 {
                    null_count += 1;
                    if null_count == 6 {
                        prefix_end = i + 1; // Start of order_label is after the 6th \0
                        break;
                    }
                }
            }

            if null_count < 6 {
                continue; // Malformed key
            }

            // Parse: order_label\0{~HLC-16bytes}\0child_id
            let after_prefix = &key[prefix_end..];

            // Find the null byte after order_label
            let order_label_end = match after_prefix.iter().position(|&b| b == 0) {
                Some(pos) => pos,
                None => continue, // Malformed key
            };

            let order_label = String::from_utf8_lossy(&after_prefix[..order_label_end]).to_string();

            // HLC starts right after the null byte (16 bytes)
            let hlc_start = order_label_end + 1;
            if after_prefix.len() < hlc_start + 16 {
                continue; // Not enough bytes for HLC
            }

            // child_id starts after HLC + one null byte
            let child_id_start = hlc_start + 16 + 1;
            if after_prefix.len() < child_id_start {
                continue; // Not enough bytes for child_id
            }

            let found_child_id =
                String::from_utf8_lossy(&after_prefix[child_id_start..]).to_string();

            // Track this (order_label, child_id) pair - skip older revisions
            let entry_key = (order_label.clone(), found_child_id.clone());
            if seen_labels.contains(&entry_key) {
                continue;
            }
            seen_labels.insert(entry_key);

            // Skip tombstones (but we've tracked them to filter older revisions)
            if is_tombstone(&value) {
                continue;
            }

            // Check if this is the child we're looking for
            if found_child_id == child_id {
                return Ok(Some(order_label));
            }
        }

        Ok(None)
    }

    /// Get order labels for TWO children efficiently (for insert-between)
    ///
    /// Returns (before_label, after_label) with O(1) targeted queries.
    /// No full child list loading!
    pub(in crate::repositories::nodes) fn get_adjacent_labels(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        before_child: Option<&str>,
        after_child: Option<&str>,
    ) -> Result<(Option<String>, Option<String>)> {
        let before_label = if let Some(child_id) = before_child {
            self.get_order_label_for_child(
                tenant_id, repo_id, branch, workspace, parent_id, child_id,
            )?
        } else {
            None
        };

        let after_label = if let Some(child_id) = after_child {
            self.get_order_label_for_child(
                tenant_id, repo_id, branch, workspace, parent_id, child_id,
            )?
        } else {
            None
        };

        Ok((before_label, after_label))
    }

    /// Get the order label of the last child (for appending)
    ///
    /// **OPTIMIZED**: Uses cached metadata for O(1) lookup instead of O(n) scan.
    /// Falls back to full scan if cache is missing (e.g., after migration).
    ///
    /// This is critical for base-36 labels where sequential appends don't maintain lex order!
    /// Example: after adding "z" then "0", we need to return "0" (newest), not "z" (lex-max).
    pub(crate) fn get_last_order_label(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
    ) -> Result<Option<String>> {
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // OPTIMIZATION: Check metadata cache first (O(1))
        let metadata_key =
            keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_id);
        if let Ok(Some(cached_value)) = self.db.get_cf(cf_ordered, &metadata_key) {
            // Cache hit! Return cached label
            return Ok(Some(String::from_utf8_lossy(&cached_value).to_string()));
        }

        // Cache miss - fall back to full scan (O(n))
        // This happens on first insert to a parent or after cache invalidation
        let prefix =
            keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_ordered, prefix);
        let mut last_label: Option<String> = None;
        let mut highest_revision = HLC::new(0, 0); // Start with minimum HLC
        let mut seen_labels = HashSet::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            // Skip tombstones
            if is_tombstone(&value) {
                continue;
            }

            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 9 {
                let order_label = String::from_utf8_lossy(parts[6]).to_string();

                // Decode ~HLC to get actual revision (parts[7] is the ~HLC encoded bytes)
                let rev_encoded = parts[7];
                if rev_encoded.len() == 16 {
                    // HLC is 16 bytes (8 bytes timestamp_ms + 8 bytes counter)
                    let revision =
                        crate::keys::decode_descending_revision(rev_encoded).map_err(|e| {
                            raisin_error::Error::storage(format!(
                                "Invalid HLC revision encoding: {}",
                                e
                            ))
                        })?;

                    // Due to ~HLC encoding, first occurrence per label has highest revision
                    // But we also need to track across ALL labels to find global max revision
                    if !seen_labels.contains(&order_label) {
                        seen_labels.insert(order_label.clone());

                        // Track label with highest HLC (most recently added)
                        if revision > highest_revision {
                            highest_revision = revision;
                            last_label = Some(order_label);
                        }
                    }
                }
            }
        }

        Ok(last_label)
    }

    /// Check if the given order_label is lexicographically >= all other children's labels
    ///
    /// This is used by reorder operations to intelligently update the metadata cache.
    /// Returns true if new_label should become the cache entry for last child.
    ///
    /// Optionally excludes a specific child_id from comparison (useful when that child
    /// is being reordered and may still have old entries in the index).
    pub(crate) fn is_lexicographically_last_label(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        test_label: &str,
        exclude_child_id: Option<&str>,
    ) -> Result<bool> {
        let prefix =
            keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_ordered, prefix);

        let mut seen_labels = HashSet::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            if is_tombstone(&value) {
                continue;
            }

            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 9 {
                let order_label = String::from_utf8_lossy(parts[6]).to_string();
                let child_id = String::from_utf8_lossy(parts[8]).to_string();

                // Skip if this is the excluded child
                if let Some(exclude_id) = exclude_child_id {
                    if child_id == exclude_id {
                        continue;
                    }
                }

                // Only check each label once (first occurrence is newest due to ~rev)
                if !seen_labels.contains(&order_label) {
                    seen_labels.insert(order_label.clone());

                    // If any label is lexicographically greater, test_label is not the last
                    if order_label.as_str() > test_label {
                        return Ok(false);
                    }
                }
            }
        }

        Ok(true)
    }

    /// Get ordered list of child IDs at HEAD (lightweight - IDs only, no node objects)
    ///
    /// Returns child IDs in order based on their order_labels.
    /// This is more efficient than loading full nodes.
    pub(in crate::repositories::nodes) async fn get_ordered_child_ids(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<String>> {
        let prefix =
            keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_ordered, prefix);

        let mut seen_entries = HashSet::new(); // Track (order_label, child_id) pairs to skip old revisions
        let mut seen_child_ids = HashSet::new(); // Track child_ids to handle reordering (same child at different labels)
        let mut child_ids = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            // Parse key: {tenant}\0{repo}\0{branch}\0{workspace}\0ORDERED_CHILDREN\0{parent_id}\0{order_label}\0{~HLC-16bytes}\0{child_id}
            //
            // IMPORTANT: Cannot use null-byte splitting for the entire key because
            // HLC's descending encoding can contain null bytes (especially counter=0).
            //
            // The prefix has 6 null-separated components:
            // tenant_id, repo_id, branch, workspace, "ORDERED_CHILDREN", parent_id
            // After the 6th \0, we have: order_label, HLC (16 bytes), child_id
            //
            // Strategy: Find the 6th null byte to locate the start of order_label

            // Find the 6th null byte (end of prefix)
            let mut null_count = 0;
            let mut prefix_end = 0;
            for (i, &byte) in key.iter().enumerate() {
                if byte == 0 {
                    null_count += 1;
                    if null_count == 6 {
                        prefix_end = i + 1; // Start of order_label is after the 6th \0
                        break;
                    }
                }
            }

            if null_count < 6 {
                // Malformed key - skip it
                continue;
            }

            // Now parse: order_label\0{~HLC-16bytes}\0child_id
            let after_prefix = &key[prefix_end..];

            // Find the null byte after order_label
            let order_label_end = match after_prefix.iter().position(|&b| b == 0) {
                Some(pos) => pos,
                None => {
                    // Malformed key - skip it
                    continue;
                }
            };

            let order_label = String::from_utf8_lossy(&after_prefix[..order_label_end]).to_string();

            // Skip metadata entries (order_label starts with special markers like ~META)
            // The descending encoding uses 0xFF for '~', which appears as � in UTF-8
            if order_label.starts_with('�')
                || order_label.contains("META")
                || order_label.contains("LAST")
                || order_label.contains("FIRST")
            {
                continue; // Skip metadata entries
            }

            // HLC starts right after the null byte
            let hlc_start = order_label_end + 1;
            if after_prefix.len() < hlc_start + 16 {
                // Not enough bytes for HLC - skip it
                continue;
            }

            let rev_bytes = &after_prefix[hlc_start..hlc_start + 16];

            // MVCC filtering: check revision before processing
            if let Some(max_rev) = max_revision {
                if let Ok(revision) = keys::decode_descending_revision(rev_bytes) {
                    if &revision > max_rev {
                        continue; // Skip revisions beyond max_revision
                    }
                }
            }

            // child_id starts after HLC + one null byte
            let child_id_start = hlc_start + 16 + 1;
            if after_prefix.len() < child_id_start {
                // Not enough bytes for child_id - skip it
                continue;
            }

            let child_id = String::from_utf8_lossy(&after_prefix[child_id_start..]).to_string();

            // Track this (order_label, child_id) pair to skip older revisions
            let entry_key = (order_label.clone(), child_id.clone());
            if seen_entries.contains(&entry_key) {
                continue;
            }
            seen_entries.insert(entry_key);

            // Skip tombstones (but we've already added them to seen_entries to filter older revisions)
            if is_tombstone(&value) {
                continue;
            }

            // Skip if we've already seen this child_id (handles reordering - same child at different labels)
            if seen_child_ids.contains(&child_id) {
                continue;
            }
            seen_child_ids.insert(child_id.clone());

            child_ids.push(child_id);
        }

        Ok(child_ids)
    }

    /// Find a child ID by name using the ordered children index
    ///
    /// This is efficient because child names are stored as values in the ordered index,
    /// avoiding the need to fetch full node objects.
    pub(in crate::repositories::nodes) fn find_child_id_by_name(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        child_name: &str,
    ) -> Result<Option<String>> {
        let prefix =
            keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_ordered, prefix);

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            // Skip tombstones
            if is_tombstone(&value) {
                continue;
            }

            // Parse child_id from key: {prefix}\0{order_label}\0{~rev}\0{child_id}
            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 9 {
                // Check if name matches (name is stored in value)
                let name_from_value = String::from_utf8_lossy(&value).to_string();
                if name_from_value == child_name {
                    let child_id = String::from_utf8_lossy(parts[8]).to_string();
                    return Ok(Some(child_id));
                }
            }
        }

        Ok(None)
    }
}
