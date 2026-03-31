//! Workspace delta operations
//!
//! Workspace deltas track uncommitted changes in a workspace before they're
//! committed to a branch. This module implements put, get, list, clear, and
//! delete operations for workspace deltas.

use super::RocksDBStorage;
use crate::cf;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::workspace::DeltaOp;

impl RocksDBStorage {
    /// Store a modified node in workspace deltas
    ///
    /// This records a "put" operation for the node at its path. The node will
    /// be persisted when the workspace is committed to the branch.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node` - The node to store in the delta
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or database operations fail
    pub async fn put_workspace_delta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
    ) -> Result<()> {
        let key = crate::keys::workspace_delta_key(
            tenant_id, repo_id, branch, workspace, "put", &node.path,
        );

        let value = rmp_serde::to_vec_named(node)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = crate::cf_handle(&self.db, cf::WORKSPACE_DELTAS)?;
        self.db.put_cf(cf, key, value).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to put workspace delta: {}", e))
        })?;

        Ok(())
    }

    /// Get a workspace delta by path
    ///
    /// Returns the modified node at the specified path if it exists in the
    /// workspace deltas.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `path` - Path to the node
    ///
    /// # Returns
    ///
    /// * `Ok(Some(node))` - Delta found
    /// * `Ok(None)` - No delta at this path
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization or database operations fail
    pub async fn get_workspace_delta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
    ) -> Result<Option<Node>> {
        let key =
            crate::keys::workspace_delta_key(tenant_id, repo_id, branch, workspace, "put", path);

        let cf = crate::cf_handle(&self.db, cf::WORKSPACE_DELTAS)?;
        let value = self.db.get_cf(cf, key).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to get workspace delta: {}", e))
        })?;

        match value {
            Some(bytes) => {
                let node = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                Ok(Some(node))
            }
            None => Ok(None),
        }
    }

    /// Get a workspace delta by node ID
    ///
    /// Scans all deltas in the workspace to find a node with the specified ID.
    /// This is less efficient than `get_workspace_delta()` but useful when you
    /// only know the node ID.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier
    ///
    /// # Returns
    ///
    /// * `Ok(Some(node))` - Delta found
    /// * `Ok(None)` - No delta with this node ID
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization or database operations fail
    pub async fn get_workspace_delta_by_id(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<Option<Node>> {
        // Scan all deltas in the workspace to find by ID
        let prefix = crate::keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("delta")
            .push("put")
            .build_prefix();

        let cf = crate::cf_handle(&self.db, cf::WORKSPACE_DELTAS)?;
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Skip invalid entries (unmigrated old data or corrupted entries)
            // Valid MessagePack Node starts with map marker, not ASCII chars
            let node: Node = match rmp_serde::from_slice(&value) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(
                        key_str = %String::from_utf8_lossy(&key),
                        first_byte = value.first().copied().unwrap_or(0),
                        error = %e,
                        "Skipping invalid delta entry (unmigrated old data?)"
                    );
                    continue; // Skip instead of fail
                }
            };

            if node.id == node_id {
                return Ok(Some(node));
            }
        }

        Ok(None)
    }

    /// List all deltas in a workspace
    ///
    /// Returns all uncommitted changes (puts and deletes) in the workspace.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    ///
    /// # Returns
    ///
    /// Vector of delta operations (Upsert or Delete)
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization or database operations fail
    pub async fn list_workspace_deltas(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<Vec<DeltaOp>> {
        let prefix = crate::keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("delta")
            .build_prefix();

        let cf = crate::cf_handle(&self.db, cf::WORKSPACE_DELTAS)?;
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut deltas = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Parse the key to determine operation type
            let key_str = String::from_utf8_lossy(&key);
            let parts: Vec<&str> = key_str.split('\0').collect();

            if parts.len() >= 6 {
                let operation = parts[5];

                if operation == "put" {
                    let node: Node = rmp_serde::from_slice(&value).map_err(|e| {
                        raisin_error::Error::storage(format!("Deserialization error: {}", e))
                    })?;
                    deltas.push(DeltaOp::Upsert(Box::new(node)));
                } else if operation == "delete" {
                    let path = parts[6..].join("\0");
                    let node_id = String::from_utf8_lossy(&value).to_string();
                    deltas.push(DeltaOp::Delete { node_id, path });
                }
            }
        }

        Ok(deltas)
    }

    /// Clear all deltas in a workspace
    ///
    /// This removes all uncommitted changes from the workspace. Typically called
    /// after committing the workspace to a branch.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail
    pub async fn clear_workspace_deltas(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<()> {
        let prefix = crate::keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("delta")
            .build_prefix();

        let cf = crate::cf_handle(&self.db, cf::WORKSPACE_DELTAS)?;
        let iter = self.db.prefix_iterator_cf(cf, prefix.clone());

        let mut keys_to_delete = Vec::new();
        for item in iter {
            let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            keys_to_delete.push(key.to_vec());
        }

        for key in keys_to_delete {
            self.db.delete_cf(cf, key).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to delete workspace delta: {}", e))
            })?;
        }

        Ok(())
    }

    /// Record a node deletion in workspace deltas
    ///
    /// This marks a node as deleted at the specified path. The deletion will be
    /// applied when the workspace is committed to the branch.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier being deleted
    /// * `path` - Path of the node being deleted
    ///
    /// # Errors
    ///
    /// Returns an error if database operations fail
    pub async fn delete_workspace_delta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        path: &str,
    ) -> Result<()> {
        let key =
            crate::keys::workspace_delta_key(tenant_id, repo_id, branch, workspace, "delete", path);

        let cf = crate::cf_handle(&self.db, cf::WORKSPACE_DELTAS)?;
        self.db.put_cf(cf, key, node_id.as_bytes()).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to delete workspace delta: {}", e))
        })?;

        // Also remove any existing "put" delta for this path
        let put_key =
            crate::keys::workspace_delta_key(tenant_id, repo_id, branch, workspace, "put", path);
        self.db.delete_cf(cf, put_key).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to delete put delta: {}", e))
        })?;

        Ok(())
    }
}
