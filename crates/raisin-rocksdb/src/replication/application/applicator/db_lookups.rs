//! Database lookup helpers: node loading, path lookups, ordering, relations, translations

use crate::{cf, cf_handle, fractional_index, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::{Node, RelationRef};
use std::collections::HashSet;

use super::{is_tombstone, OperationApplicator};

impl OperationApplicator {
    /// Load the latest version of a node from RocksDB
    pub(in crate::replication::application) fn load_latest_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_id: &str,
    ) -> Result<Option<Node>> {
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .build_prefix();

        let mut iter = self.db.iterator_cf(
            cf_nodes,
            rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
        );

        while let Some(Ok((key, value))) = iter.next() {
            if !key.starts_with(&prefix) {
                break;
            }

            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() < 6 {
                continue;
            }

            if parts[5] == node_id.as_bytes() {
                if is_tombstone(&value) {
                    return Ok(None);
                }

                let node: Node = rmp_serde::from_slice(&value).map_err(|e| {
                    raisin_error::Error::storage(format!(
                        "Failed to deserialize node during delete: {}",
                        e
                    ))
                })?;
                return Ok(Some(node));
            }
        }

        Ok(None)
    }

    /// Resolve parent ID from a node's path
    pub(in crate::replication::application) fn resolve_parent_id_for_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
    ) -> Result<Option<String>> {
        if let Some(parent_path) = node.parent_path() {
            if parent_path == "/" {
                return Ok(Some("/".to_string()));
            }
            return self.lookup_node_id_by_path(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &parent_path,
            );
        }
        Ok(None)
    }

    /// Look up a node ID by its path in the PATH_INDEX
    pub(in crate::replication::application) fn lookup_node_id_by_path(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
    ) -> Result<Option<String>> {
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("path")
            .push(path)
            .build_prefix();
        let prefix_clone = prefix.clone();
        let mut iter = self.db.prefix_iterator_cf(cf_path, &prefix);

        if let Some(Ok((key, value))) = iter.next() {
            if key.starts_with(&prefix_clone) && !is_tombstone(&value) {
                return Ok(Some(String::from_utf8_lossy(&value).into_owned()));
            }
        }

        Ok(None)
    }

    /// Allocate a new order label for inserting a child after the last existing child
    pub(in crate::replication::application) fn allocate_order_label(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
    ) -> Result<String> {
        if let Some(last_label) =
            self.get_last_order_label_for_parent(tenant_id, repo_id, branch, workspace, parent_id)?
        {
            let last_fractional = fractional_index::extract_fractional(&last_label);
            fractional_index::inc(last_fractional)
        } else {
            Ok(fractional_index::first())
        }
    }

    /// Get the last order label for a parent, checking the metadata cache first
    pub(in crate::replication::application) fn get_last_order_label_for_parent(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
    ) -> Result<Option<String>> {
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let metadata_key =
            keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_id);

        if let Ok(Some(cached_value)) = self.db.get_cf(cf_ordered, &metadata_key) {
            if let Some(label) = Self::parse_order_label(&cached_value) {
                return Ok(Some(label));
            } else {
                tracing::warn!(
                    parent_id = %parent_id,
                    "⚠️ Ignoring invalid cached order label; resetting metadata"
                );
                self.db
                    .delete_cf(cf_ordered, &metadata_key)
                    .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            }
        }

        self.scan_last_order_label_for_parent(tenant_id, repo_id, branch, workspace, parent_id)
    }

    /// Scan ORDERED_CHILDREN to find the last order label for a parent
    pub(in crate::replication::application) fn scan_last_order_label_for_parent(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
    ) -> Result<Option<String>> {
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let prefix =
            keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_ordered, prefix);

        let mut last_label: Option<String> = None;
        let mut highest_revision = HLC::new(0, 0);
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
                if let Some(order_label) = Self::parse_order_label(parts[6]) {
                    let revision_bytes = parts[7];
                    if revision_bytes.len() == 16 {
                        let revision =
                            keys::decode_descending_revision(revision_bytes).map_err(|e| {
                                raisin_error::Error::storage(format!(
                                    "Invalid HLC revision encoding: {}",
                                    e
                                ))
                            })?;

                        if seen_labels.insert(order_label.clone()) && revision > highest_revision {
                            highest_revision = revision;
                            last_label = Some(order_label);
                        }
                    }
                }
            }
        }

        Ok(last_label)
    }

    /// Parse and validate an order label from raw bytes
    pub(in crate::replication::application) fn parse_order_label(raw: &[u8]) -> Option<String> {
        if raw.is_empty() {
            return None;
        }
        let label = String::from_utf8_lossy(raw).to_string();
        if label.is_empty() {
            return None;
        }

        let fractional_part = fractional_index::extract_fractional(&label);

        if ::fractional_index::FractionalIndex::from_string(fractional_part).is_err() {
            return None;
        }

        // Return the full label (including compound suffix if present)
        Some(label)
    }

    /// Collect incoming relations for a node
    pub(in crate::replication::application) fn collect_incoming_relations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let prefix = keys::relation_reverse_prefix(tenant_id, repo_id, branch, workspace, node_id);
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_relation, &prefix);

        let mut relations = Vec::new();
        let mut seen = HashSet::new();

        for item in iter {
            let (key, value) =
                item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            if is_tombstone(&value) {
                continue;
            }

            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 9 {
                let relation_type = String::from_utf8_lossy(parts[6]).to_string();
                let source_node_id = String::from_utf8_lossy(parts[8]).to_string();
                let dedupe_key = (source_node_id.clone(), relation_type.clone());
                if seen.insert(dedupe_key) {
                    relations.push((source_node_id, relation_type, workspace.to_string()));
                }
            }
        }

        Ok(relations)
    }

    /// Collect outgoing relations for a node
    pub(in crate::replication::application) fn collect_outgoing_relations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<Vec<RelationRef>> {
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let prefix = keys::relation_forward_prefix(tenant_id, repo_id, branch, workspace, node_id);
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_relation, &prefix);

        let mut relations = Vec::new();
        let mut seen = HashSet::new();

        for item in iter {
            let (key, value) =
                item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            if is_tombstone(&value) {
                continue;
            }

            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 9 {
                let relation_type = String::from_utf8_lossy(parts[6]).to_string();
                let target_node_id = String::from_utf8_lossy(parts[8]).to_string();
                let dedupe_key = (target_node_id.clone(), relation_type.clone());
                if seen.insert(dedupe_key) {
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

    /// List all translation locales for a node
    pub(in crate::replication::application) fn list_translation_locales(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<Vec<String>> {
        let cf_translation = cf_handle(&self.db, cf::TRANSLATION_DATA)?;
        let prefix = format!(
            "{}\0{}\0{}\0{}\0translations\0{}\0",
            tenant_id, repo_id, branch, workspace, node_id
        )
        .into_bytes();
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_translation, &prefix);
        let mut locales = HashSet::new();

        for item in iter {
            let (key, value) =
                item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            if is_tombstone(&value) {
                continue;
            }

            let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if parts.len() >= 7 {
                locales.insert(String::from_utf8_lossy(parts[6]).into_owned());
            }
        }

        Ok(locales.into_iter().collect())
    }
}
