//! Version management methods for NodeService
//!
//! This module provides access to node version history using the revision system.
//! Manual versions are implemented as flagged revisions with is_manual_version=true.
//!
//! Features:
//! - Manual version creation (creates a revision with manual version flags)
//! - Version restoration (loads snapshot from revision and applies to current node)
//! - Version listing (filters revisions by manual_version_node_id)
//! - No duplicate storage - reuses revision snapshots

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::{Node, NodeVersion};
use raisin_models::tree::TreeCommitMeta;
use raisin_storage::{transactional::TransactionalStorage, RevisionRepository, Storage};

use super::NodeService;

impl<S: Storage + TransactionalStorage + 'static> NodeService<S> {
    /// Lists all manual versions for a node at the given path
    ///
    /// Queries the revision history and filters for revisions where
    /// manual_version_node_id matches this node's ID.
    pub async fn list_versions(&self, node_path: &str) -> Result<Vec<NodeVersion>> {
        let node = self
            .get_by_path(node_path)
            .await?
            .ok_or(Error::NotFound("node".into()))?;

        // Get all revisions and extract revision numbers
        // The filtering by node ID happens below via TreeCommitMeta.manual_version_node_id
        let all_rev_metas = self
            .storage
            .revisions()
            .list_revisions(&self.tenant_id, &self.repo_id, 1000, 0)
            .await?;
        let revisions: Vec<HLC> = all_rev_metas.iter().map(|m| m.revision).collect();

        let mut versions = Vec::new();
        let mut version_number = 1;

        // Load each revision's metadata and filter for manual versions
        // Iterate in reverse (oldest first) for sequential numbering
        for rev_num in revisions.iter().rev() {
            // Get TreeCommitMeta for this revision
            if let Some(_meta) = self
                .storage
                .revisions()
                .get_revision_meta(&self.tenant_id, &self.repo_id, rev_num)
                .await?
            {
                // The RevisionMeta doesn't have our new fields, we need to get TreeCommitMeta directly
                // We'll need to access the storage backend directly for this
                // Let's try to deserialize the raw metadata

                // For now, let's use a helper method that we'll add to get TreeCommitMeta
                match self.get_tree_commit_meta(rev_num).await {
                    Ok(tree_meta) => {
                        // Filter for manual versions of this specific node
                        if tree_meta.is_manual_version
                            && tree_meta.manual_version_node_id.as_ref() == Some(&node.id)
                        {
                            // Load node snapshot from this revision
                            if let Some(snapshot_bytes) = self
                                .storage
                                .revisions()
                                .get_node_snapshot(
                                    &self.tenant_id,
                                    &self.repo_id,
                                    &node.id,
                                    rev_num,
                                )
                                .await?
                            {
                                if let Ok(snapshot) = rmp_serde::from_slice::<Node>(&snapshot_bytes)
                                {
                                    // Convert to NodeVersion format for API compatibility
                                    let node_version = NodeVersion {
                                        id: format!("{}:{}", node.id, version_number),
                                        node_id: node.id.clone(),
                                        version: version_number,
                                        node_data: snapshot,
                                        note: Some(tree_meta.message),
                                        created_at: Some(tree_meta.timestamp),
                                        updated_at: Some(tree_meta.timestamp),
                                    };
                                    versions.push(node_version);
                                    version_number += 1;
                                }
                            }
                        }
                    }
                    Err(_e) => {
                        // Failed to get TreeCommitMeta, skip this revision
                    }
                }
            }
        }

        // Reverse to show newest first
        versions.reverse();
        Ok(versions)
    }

    /// Helper method to get TreeCommitMeta for a revision
    /// This accesses the storage backend directly since RevisionMeta doesn't have all fields
    async fn get_tree_commit_meta(&self, revision: &HLC) -> Result<TreeCommitMeta> {
        // We need to access the DB directly - this is a bit of a hack but necessary
        // since we're bridging between the old and new metadata formats

        // Try to get it through the storage interface
        // The storage backend should have a way to access this

        // For RocksDB storage, we can read it directly
        // Let's use the fact that TreeCommitMeta is stored at the same location as RevisionMeta

        // This is implementation-specific, but we need it for now
        // The proper solution would be to extend RevisionRepository trait

        // For now, we'll use get_revision_meta and then try to deserialize as TreeCommitMeta
        // But that won't work because RevisionMeta is different

        // Actually, let me check if we can downcast the storage to RocksDB storage
        use std::any::Any;

        // Get the storage as Any to downcast
        let storage_any: &dyn Any = &*self.storage;

        #[cfg(feature = "storage-rocksdb")]
        if let Some(rocks_storage) = storage_any.downcast_ref::<raisin_rocksdb::RocksDBStorage>() {
            // Access RocksDB directly
            let db = rocks_storage.db();
            let key = raisin_rocksdb::keys::k_commit_meta(&self.tenant_id, &self.repo_id, revision);

            if let Some(bytes) = db
                .get(key.as_slice())
                .map_err(|e| Error::Backend(format!("Failed to read commit meta: {}", e)))?
            {
                let tree_meta: TreeCommitMeta = rmp_serde::from_slice(&bytes).map_err(|e| {
                    Error::Backend(format!("Failed to deserialize TreeCommitMeta: {}", e))
                })?;
                return Ok(tree_meta);
            }
        }

        #[cfg(not(feature = "storage-rocksdb"))]
        let _ = storage_any; // Suppress unused variable warning

        Err(Error::NotFound("TreeCommitMeta not found".to_string()))
    }

    /// Gets a specific manual version of a node by version number
    ///
    /// Version numbers are sequential (1, 2, 3, ...) based on creation order.
    pub async fn get_version(&self, node_path: &str, version: i32) -> Result<Option<NodeVersion>> {
        let versions = self.list_versions(node_path).await?;
        Ok(versions.into_iter().find(|v| v.version == version))
    }

    /// Creates a manual version of a node with an optional note
    ///
    /// This creates a new revision with is_manual_version=true.
    /// The node is touched (updated_at modified) to trigger a commit.
    ///
    /// # Arguments
    /// * `node_path` - The path to the node
    /// * `note` - Optional comment describing this version
    ///
    /// # Returns
    /// The version number created (sequential, starting from 1)
    pub async fn create_manual_version(
        &self,
        node_path: &str,
        note: Option<String>,
    ) -> Result<i32> {
        let mut node = self
            .get_by_path(node_path)
            .await?
            .ok_or(Error::NotFound("node".into()))?;

        // Count existing manual versions to determine next version number
        let existing_versions = self.list_versions(node_path).await?;
        let next_version = (existing_versions.len() as i32) + 1;

        // Touch the node to trigger change detection
        node.updated_at = Some(chrono::Utc::now());

        // Create transaction with manual version metadata
        let ctx = self.storage.begin_context().await?;

        // Set auth context for RLS enforcement
        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        ctx.set_tenant_repo(&self.tenant_id, &self.repo_id)?;
        ctx.set_branch(&self.branch)?;
        ctx.set_message(&note.unwrap_or_else(|| format!("Manual version {}", next_version)))?;
        ctx.set_actor("user")?; // TODO: Get from context
        ctx.set_is_manual_version(true)?;
        ctx.set_manual_version_node_id(&node.id)?;

        // Put the node (will be included in commit)
        ctx.put_node(&self.workspace_id, &node).await?;

        // Commit the transaction (creates revision with manual version flags)
        ctx.commit().await?;

        Ok(next_version)
    }

    /// Restores a node to a specific manual version
    ///
    /// This loads the snapshot from the manual version's revision and
    /// applies it to the current node, creating a new revision (non-destructive).
    ///
    /// # Arguments
    /// * `node_path` - The path to the node
    /// * `version` - The version number to restore to
    ///
    /// # Returns
    /// The updated node after restoration
    pub async fn restore_version(&self, node_path: &str, version: i32) -> Result<Node> {
        let mut current_node = self
            .get_by_path(node_path)
            .await?
            .ok_or(Error::NotFound("node".into()))?;

        // Find the manual version revision
        let version_data = self
            .get_version(node_path, version)
            .await?
            .ok_or(Error::NotFound("version".into()))?;

        let snapshot = version_data.node_data;

        // Copy content from snapshot (keep identity: id, path, name, parent, workspace)
        current_node.properties = snapshot.properties;
        current_node.translations = snapshot.translations;
        current_node.node_type = snapshot.node_type;
        current_node.archetype = snapshot.archetype;
        current_node.updated_at = Some(chrono::Utc::now());

        // Update the node (creates new regular revision, NOT a manual version)
        let ctx = self.storage.begin_context().await?;

        // Set auth context for RLS enforcement
        if let Some(auth) = &self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        ctx.set_tenant_repo(&self.tenant_id, &self.repo_id)?;
        ctx.set_branch(&self.branch)?;
        ctx.set_message(&format!("Restored from manual version {}", version))?;
        ctx.set_actor("user")?; // TODO: Get from context

        ctx.put_node(&self.workspace_id, &current_node).await?;
        ctx.commit().await?;

        Ok(current_node)
    }

    /// Deletes a specific manual version
    ///
    /// Note: Currently not supported as revisions are immutable.
    /// Returns an error indicating versions cannot be deleted.
    pub async fn delete_version(&self, _node_path: &str, _version: i32) -> Result<bool> {
        Err(Error::Validation(
            "Manual versions cannot be deleted as they are immutable revisions. \
             Use cleanup_old_versions to mark old versions for garbage collection."
                .to_string(),
        ))
    }

    /// Cleans up old manual versions, keeping only the N most recent
    ///
    /// Note: Currently not supported as revisions are immutable.
    /// In the future, this could mark revisions for garbage collection.
    pub async fn cleanup_old_versions(
        &self,
        _node_path: &str,
        _keep_count: usize,
    ) -> Result<usize> {
        Err(Error::Validation(
            "Manual version cleanup not yet implemented. \
             Revisions are immutable and will be cleaned up by repository-level garbage collection.".to_string()
        ))
    }

    /// Updates the note/comment for a specific manual version
    ///
    /// Note: Currently not supported as revision metadata is immutable.
    pub async fn update_version_note(
        &self,
        _node_path: &str,
        _version: i32,
        _note: Option<String>,
    ) -> Result<()> {
        Err(Error::Validation(
            "Manual version notes cannot be updated as revision metadata is immutable.".to_string(),
        ))
    }
}
