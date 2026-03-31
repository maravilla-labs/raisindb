use super::helpers::{
    deserialize_compact_relations, get_relation_cf, serialize_compact_relations, CompactRelation,
};
use crate::keys::node_adjacency_key_versioned;
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::RelationRef;
use rocksdb::DB;
use std::sync::Arc;

/// Repository for managing packed adjacency lists
///
/// This repository implements the "Packed Adjacency" storage optimization where
/// all outgoing edges for a node are stored in a single RocksDB key.
/// This reduces storage overhead and allows O(1) retrieval of all neighbors.
#[derive(Clone)]
pub struct PackedRelationRepository {
    db: Arc<DB>,
}

impl PackedRelationRepository {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Add a relation to the packed adjacency list
    pub async fn add_relation(
        &self,
        revision: &HLC,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        source_node_id: &str,
        relation: &RelationRef,
    ) -> Result<()> {
        // 1. Read the most recent version of the adjacency list
        let prev_relations = self
            .get_packed_relations(
                revision,
                tenant_id,
                repo_id,
                branch,
                workspace,
                source_node_id,
            )
            .await?;

        let mut new_relations = prev_relations.unwrap_or_default();

        // Check if relation already exists (update it) or append
        let compact = CompactRelation {
            relation_type: relation.relation_type.clone(),
            target_id: relation.target.clone(),
            target_workspace: relation.workspace.clone(),
            target_node_type: relation.target_node_type.clone(),
            weight: relation.weight,
        };

        if let Some(idx) = new_relations.iter().position(|r| {
            r.relation_type == compact.relation_type && r.target_id == compact.target_id
        }) {
            new_relations[idx] = compact;
        } else {
            new_relations.push(compact);
        }

        // Serialize and write
        let bytes = serialize_compact_relations(&new_relations)?;
        let key = node_adjacency_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            source_node_id,
            revision,
        );

        let cf = get_relation_cf(&self.db)?;
        self.db
            .put_cf(cf, key, bytes)
            .map_err(|e| Error::storage(format!("Failed to write packed relations: {}", e)))?;

        Ok(())
    }

    /// Remove a relation from the packed adjacency list
    pub async fn remove_relation(
        &self,
        revision: &HLC,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        source_node_id: &str,
        _target_workspace: &str, // Ignored for packed storage (assumes same workspace or stored in target_id)
        target_node_id: &str,
    ) -> Result<bool> {
        // Read previous state
        let prev_relations = self
            .get_packed_relations(
                revision,
                tenant_id,
                repo_id,
                branch,
                workspace,
                source_node_id,
            )
            .await?;

        if let Some(mut relations) = prev_relations {
            let initial_len = relations.len();

            // Remove relations matching target_node_id
            relations.retain(|r| r.target_id != target_node_id);

            if relations.len() != initial_len {
                // Write new version
                let bytes = serialize_compact_relations(&relations)?;
                let key = node_adjacency_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    source_node_id,
                    revision,
                );

                let cf = get_relation_cf(&self.db)?;
                self.db.put_cf(cf, key, bytes).map_err(|e| {
                    Error::storage(format!("Failed to write packed relations: {}", e))
                })?;
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Get packed relations for a node at a specific revision
    pub async fn get_packed_relations(
        &self,
        max_revision: &HLC,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<Option<Vec<CompactRelation>>> {
        let cf = get_relation_cf(&self.db)?;

        // We scan for the first key for this node that has revision <= max_revision
        // Newer revisions sort first (smaller key due to ~rev encoding)
        // So we seek to the key constructed with max_revision.
        // If we find a key, it will have ~rev >= ~max_rev (so rev <= max_rev).

        let key = node_adjacency_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            max_revision,
        );

        let mut iter = self.db.raw_iterator_cf(cf);
        iter.seek(&key);

        if iter.valid() {
            if let Some(k) = iter.key() {
                // Check if it's still for the same node
                // The prefix up to "adj" should match.
                // The revision is the last 16 bytes.
                let prefix_len = key.len() - 16;

                if k.len() >= prefix_len && k[..prefix_len] == key[..prefix_len] {
                    if let Some(v) = iter.value() {
                        let relations = deserialize_compact_relations(v)?;
                        return Ok(Some(relations));
                    }
                }
            }
        }

        Ok(None)
    }
}

// Keep standalone functions for backward compatibility if needed,
// or remove them if we update all call sites.
// For now, we'll remove them to force usage of the struct.
