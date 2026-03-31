//! Helper functions for CRUD operations
//!
//! This module contains shared utility functions used across CRUD operations:
//! - Revision lookups (latest, at-or-before)
//! - Relation queries (outgoing, incoming)
//! - Translation locale listing
//! - Tree ordering helpers

use super::super::helpers::{is_tombstone, TOMBSTONE};
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::RelationRef;
use std::collections::HashSet;

impl NodeRepositoryImpl {
    /// Get the latest revision number for a node
    ///
    /// Scans the NODES CF prefix for the given node and returns the newest revision
    /// (due to descending revision encoding, the first item is the newest).
    ///
    /// # Returns
    /// - `Some(revision)` if the node has at least one revision
    /// - `None` if the node doesn't exist at any revision
    pub(in super::super) fn get_latest_revision_for_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<Option<HLC>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .push(node_id)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::NODES)?;
        let mut iter = self.db.prefix_iterator_cf(cf, prefix);

        // Due to descending revision encoding, the first item is the newest
        if let Some(item) = iter.next() {
            let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            let revision = keys::extract_revision_from_key(&key).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to decode revision: {}", e))
            })?;
            return Ok(Some(revision));
        }

        Ok(None)
    }

    /// Get the latest revision for a node at or before a target revision
    ///
    /// Used for time-travel queries to find the most recent version of a node
    /// that exists at or before a specific revision.
    ///
    /// # Returns
    /// - `Some(revision)` if the node existed at or before target_revision
    /// - `None` if the node didn't exist at or before target_revision
    pub(in super::super) fn get_revision_at_or_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        target_revision: &HLC,
    ) -> Result<Option<HLC>> {
        tracing::debug!(
            target: "rocksb::nodes::revision_lookup",
            "get_revision_at_or_before: tenant={} repo={} branch={} workspace={} node_id={} target_revision={}",
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            target_revision
        );
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .push(node_id)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::NODES)?;
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        // Iterate through revisions (newest first due to descending encoding)
        // Return the first revision that is <= target_revision
        for item in iter {
            let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            let revision = match keys::extract_revision_from_key(&key) {
                Ok(rev) => rev,
                Err(e) => {
                    tracing::warn!(
                        target: "rocksb::nodes::revision_lookup",
                        "Skipping key with invalid revision for node_id={}: {}",
                        node_id,
                        e
                    );
                    continue;
                }
            };

            if &revision <= target_revision {
                tracing::debug!(
                    target: "rocksb::nodes::revision_lookup",
                    "revision candidate found: node_id={} candidate={} target={}",
                    node_id,
                    revision,
                    target_revision
                );
                return Ok(Some(revision));
            }
        }

        tracing::debug!(
            target: "rocksb::nodes::revision_lookup",
            "no revision found at or before target: node_id={} target={}",
            node_id,
            target_revision
        );

        Ok(None)
    }

    /// Get all outgoing relations from a node (where THIS node points TO other nodes)
    ///
    /// Queries the RELATION_INDEX CF forward index to find all relations
    /// where this node is the source.
    ///
    /// # Returns
    /// Vector of RelationRef objects representing outgoing relations
    pub(in super::super) fn get_outgoing_relations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        source_node_id: &str,
    ) -> Result<Vec<RelationRef>> {
        // Use the correct prefix function that matches how relations are stored
        let prefix =
            keys::relation_forward_prefix(tenant_id, repo_id, branch, workspace, source_node_id);
        let prefix_clone = prefix.clone();

        let cf = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut relations = Vec::new();
        let mut seen: HashSet<(String, String)> = HashSet::new(); // (target_node_id, relation_type)

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            // Parse key to extract target_node_id and relation_type
            // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0rel\0{source_node_id}\0{relation_type}\0{~revision-16bytes}\0{target_node_id}
            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 9 {
                let relation_type = String::from_utf8_lossy(parts[6]).to_string();
                let target_node_id = String::from_utf8_lossy(parts[8]).to_string();

                // Since keys are sorted by ~revision (newest first), we only want the first occurrence
                let pair_key = (target_node_id.clone(), relation_type.clone());
                if !seen.contains(&pair_key) {
                    // Mark as seen BEFORE checking tombstone - this ensures we don't
                    // return older versions if the latest is a tombstone
                    seen.insert(pair_key);

                    // Check if this is a tombstone (deleted relation)
                    if is_tombstone(&value) {
                        continue;
                    }

                    // Deserialize the RelationRef to get full relation details
                    let relation: RelationRef = rmp_serde::from_slice(&value).map_err(|e| {
                        raisin_error::Error::storage(format!(
                            "Failed to deserialize relation: {}",
                            e
                        ))
                    })?;

                    relations.push(relation);
                }
            }
        }

        Ok(relations)
    }

    /// Get all incoming relations to a node (where other nodes point TO this node)
    ///
    /// Queries the RELATION_INDEX CF reverse index to find all relations
    /// where this node is the target.
    ///
    /// # Returns
    /// Vector of tuples: (source_node_id, relation_type, source_workspace)
    pub(in super::super) fn get_incoming_relations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        target_node_id: &str,
    ) -> Result<Vec<(String, String, String)>> {
        // Use the correct prefix function that matches how relations are stored
        let prefix =
            keys::relation_reverse_prefix(tenant_id, repo_id, branch, workspace, target_node_id);
        let prefix_clone = prefix.clone();

        let cf = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut relations = Vec::new();
        let mut seen: HashSet<(String, String, String)> = HashSet::new(); // (source, type, workspace)

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            // Parse key to extract source_node_id and relation_type
            // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0rel_rev\0{target_node_id}\0{relation_type}\0{~revision-16bytes}\0{source_node_id}
            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 9 {
                let source_workspace = String::from_utf8_lossy(parts[3]).to_string();
                let relation_type = String::from_utf8_lossy(parts[6]).to_string();
                let source_node_id = String::from_utf8_lossy(parts[8]).to_string();

                // Since keys are sorted by ~revision (newest first), we only want the first occurrence
                let triple_key = (
                    source_node_id.clone(),
                    relation_type.clone(),
                    source_workspace.clone(),
                );
                if !seen.contains(&triple_key) {
                    // Mark as seen BEFORE checking tombstone - this ensures we don't
                    // return older versions if the latest is a tombstone
                    seen.insert(triple_key);

                    // Check if this is a tombstone (deleted relation)
                    if is_tombstone(&value) {
                        continue;
                    }

                    relations.push((source_node_id, relation_type, source_workspace));
                }
            }
        }

        Ok(relations)
    }

    /// List all translation locales for a node
    ///
    /// Scans the TRANSLATION_DATA CF to find all locales that have translations
    /// for the given node.
    ///
    /// # Returns
    /// Vector of locale codes (e.g., ["en", "de", "fr"])
    pub(in super::super) fn list_translation_locales(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<Vec<String>> {
        let prefix = format!(
            "{}\0{}\0{}\0{}\0translations\0{}\0",
            tenant_id, repo_id, branch, workspace, node_id
        );

        let cf = cf_handle(&self.db, cf::TRANSLATION_DATA)?;
        let iter = self.db.prefix_iterator_cf(cf, prefix.as_bytes());

        let mut locales = HashSet::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Skip tombstones
            if &*value == TOMBSTONE {
                continue;
            }

            // Parse locale from key
            // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0translations\0{node_id}\0{locale}\0{~revision}
            let key_str = String::from_utf8_lossy(&key);
            let parts: Vec<&str> = key_str.split('\0').collect();
            if parts.len() >= 7 {
                let locale = parts[6].to_string();
                locales.insert(locale);
            }
        }

        Ok(locales.into_iter().collect())
    }
}
