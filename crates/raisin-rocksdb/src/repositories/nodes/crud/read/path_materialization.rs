//! Path materialization from NODE_PATH index
//!
//! Since nodes are stored as StorageNode (without path), the path must be
//! materialized from the NODE_PATH index during reads. This module handles
//! that process and backward compatibility with old Node format.

use super::super::super::helpers::is_tombstone;
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// Materialize the path for a node from the NODE_PATH index
    ///
    /// This is used when reading nodes stored as StorageNode (without path).
    /// For backward compatibility, if the node already has a path (old data),
    /// this function is not called.
    pub(in crate::repositories::nodes) fn materialize_path(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        target_revision: &HLC,
    ) -> Result<String> {
        let prefix = keys::node_path_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
        let cf = cf_handle(&self.db, cf::NODE_PATH)?;

        let iter = self.db.prefix_iterator_cf(cf, prefix.clone());

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix) {
                break;
            }

            let revision = match keys::extract_revision_from_key(&key) {
                Ok(rev) => rev,
                Err(_) => continue,
            };

            // Skip revisions beyond target_revision (due to descending encoding, newest first)
            if &revision > target_revision {
                continue;
            }

            // Check for tombstone - node was deleted at this revision
            if is_tombstone(&value) {
                return Err(raisin_error::Error::storage(format!(
                    "Node {} was deleted (tombstone in NODE_PATH)",
                    node_id
                )));
            }

            let path = String::from_utf8(value.to_vec()).map_err(|e| {
                raisin_error::Error::storage(format!("Invalid path encoding: {}", e))
            })?;

            return Ok(path);
        }

        Err(raisin_error::Error::storage(format!(
            "Path not found for node_id={} at revision={}",
            node_id, target_revision
        )))
    }

    /// Deserialize a node from bytes and materialize path if needed
    ///
    /// This handles backward compatibility:
    /// - New data: StorageNode without path -> materialize path from NODE_PATH index
    /// - Old data: Node with path -> falls back to Node deserialization
    pub(in crate::repositories::nodes) fn deserialize_node_with_path(
        &self,
        bytes: &[u8],
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        target_revision: &HLC,
    ) -> Result<Node> {
        use super::super::super::storage_node::StorageNode;

        // First, try to deserialize as StorageNode (new format without path)
        if let Ok(storage_node) = rmp_serde::from_slice::<StorageNode>(bytes) {
            match self.materialize_path(
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_id,
                target_revision,
            ) {
                Ok(path) => {
                    tracing::debug!(
                        "Deserialized StorageNode, materialized path for node_id={}: {}",
                        node_id,
                        path
                    );
                    return Ok(storage_node.into_node(path));
                }
                Err(_) => {
                    tracing::debug!(
                        "Path materialization failed for node_id={}, trying Node format",
                        node_id
                    );
                }
            }
        }

        // Fallback: try to deserialize as Node (old format with path)
        let node: Node = rmp_serde::from_slice(bytes)
            .map_err(|e| raisin_error::Error::storage(format!("Deserialization error: {}", e)))?;

        tracing::trace!(
            "Deserialized old Node format, path already present for node_id={}: {}",
            node_id,
            node.path
        );
        Ok(node)
    }
}
