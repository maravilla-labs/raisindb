//! Path materialization from NODE_PATH index
//!
//! Since nodes are stored as StorageNode (without path), the path must be
//! materialized from the NODE_PATH index during reads. This module handles
//! that process and backward compatibility with old Node format.

use super::super::super::storage_node::PropertiesMode;
use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

impl NodeRepositoryImpl {
    /// Materialize the path for a node from the NODE_PATH index
    ///
    /// This is used when reading nodes stored as StorageNode (without path).
    /// For backward compatibility, if the node already has a path (old data),
    /// this function is not called.
    /// Visible crate-wide because the INDEX REBUILDS need it too: they iterate
    /// the node blobs directly, and a blob deliberately carries no `path`.
    ///
    /// One seek, shared with the transaction read path: see
    /// [`crate::mvcc_read::materialize_path`].
    pub(crate) fn materialize_path(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        target_revision: &HLC,
    ) -> Result<String> {
        crate::mvcc_read::materialize_path(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            target_revision,
        )
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
        self.deserialize_node_with_path_as(
            bytes,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            target_revision,
            PropertiesMode::Load,
        )
    }

    /// [`Self::deserialize_node_with_path`], optionally without decoding the
    /// properties (`PropertiesMode::Skip` returns an empty map).
    ///
    /// The legacy full-`Node` fallback always decodes everything: it is rare,
    /// and correctness there is worth more than the saving.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) fn deserialize_node_with_path_as(
        &self,
        bytes: &[u8],
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        target_revision: &HLC,
        mode: PropertiesMode,
    ) -> Result<Node> {
        use super::super::super::storage_node::{StorageNode, StorageNodeHead};

        if mode == PropertiesMode::Skip {
            // The head reader accepts everything the full one does, and its
            // path lookup is the same one: if either fails here it would fail
            // below too, so go straight to the legacy format instead of paying
            // for the StorageNode attempt twice.
            if let Ok(head) = rmp_serde::from_slice::<StorageNodeHead>(bytes) {
                if let Ok(path) = self.materialize_path(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    node_id,
                    target_revision,
                ) {
                    return Ok(head.into_node_without_properties(path));
                }
            }
        } else if let Ok(storage_node) = rmp_serde::from_slice::<StorageNode>(bytes) {
            // First, try to deserialize as StorageNode (new format without path)
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
