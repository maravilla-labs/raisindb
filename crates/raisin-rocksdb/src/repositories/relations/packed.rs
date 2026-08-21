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
            r.relation_type == compact.relation_type
                && r.target_id == compact.target_id
                && r.target_workspace == compact.target_workspace
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
        target_workspace: &str,
        target_node_id: &str,
        relation_type: Option<&str>,
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

            // Remove relations matching the target (id AND workspace — the same
            // id can exist in two workspaces) — and, when a type is given, only
            // that type. A CompactRelation carries its own
            // relation_type, so the pair's other edges are kept.
            relations.retain(|r| {
                let same_target =
                    r.target_id == target_node_id && r.target_workspace == target_workspace;
                let same_type = relation_type.is_none_or(|want| want == r.relation_type);
                !(same_target && same_type)
            });

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
        super::helpers::read_packed_relations_at(
            &self.db,
            max_revision,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
        )
    }
}

// Keep standalone functions for backward compatibility if needed,
// or remove them if we update all call sites.
// For now, we'll remove them to force usage of the struct.
