//! Tree navigation helpers.
//!
//! Internal helpers for navigating the Merkle-like tree structure
//! to find children tree IDs and tree entries for given paths.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_storage::{scope::RepoScope, NodeRepository, Storage, TreeRepository};

use super::super::NodeService;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage> NodeService<S> {
    /// Helper: Navigate tree to find children_tree_id for a given path
    pub(in crate::services::node_service) async fn find_children_tree_id_for_path(
        &self,
        root_tree_id: &[u8; 32],
        path: &str,
        revision: &HLC,
    ) -> Result<Option<[u8; 32]>> {
        // Parse path into components
        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if components.is_empty() {
            // Root path
            return Ok(Some(*root_tree_id));
        }

        // Start from root tree
        let mut current_tree_id = *root_tree_id;

        // Navigate through each path component
        for (idx, component) in components.iter().enumerate() {
            // Get entries from current tree
            let entries = self
                .storage
                .trees()
                .iter_tree(
                    RepoScope::new(&self.tenant_id, &self.repo_id),
                    &current_tree_id,
                    None,
                    10000,
                )
                .await?;

            // Find entry matching this component
            // At root level, entry_key = node_id, so we need to find by name
            // At child levels, entry_key = node_name
            let matching_entry = if idx == 0 {
                // Root level: need to check node from NODES CF to match by name
                let mut found_entry = None;
                for entry in &entries {
                    if let Some(node) = self
                        .storage
                        .nodes()
                        .get(self.scope(), &entry.node_id, Some(revision))
                        .await?
                    {
                        if node.name == *component {
                            found_entry = Some(entry.clone());
                            break;
                        }
                    }
                }
                found_entry
            } else {
                // Child level: entry_key = node_name
                entries.into_iter().find(|e| e.entry_key == *component)
            };

            match matching_entry {
                Some(entry) => {
                    // If this is the last component, return its children_tree_id
                    if idx == components.len() - 1 {
                        return Ok(entry.children_tree_id);
                    }

                    // Otherwise, navigate to its children tree
                    if let Some(children_tree_id) = entry.children_tree_id {
                        current_tree_id = children_tree_id;
                    } else {
                        // Node exists but has no children
                        return Ok(None);
                    }
                }
                None => {
                    // Path component not found
                    return Err(raisin_error::Error::NotFound(format!(
                        "Path component '{}' not found in path '{}'",
                        component, path
                    )));
                }
            }
        }

        Ok(None)
    }

    /// Helper: Navigate tree to find the TreeEntry for a given path
    ///
    /// Similar to `find_children_tree_id_for_path`, but returns the full TreeEntry
    /// instead of just the children_tree_id. Useful for checking if a node has children.
    pub(crate) async fn find_tree_entry_for_path(
        &self,
        root_tree_id: &[u8; 32],
        path: &str,
        revision: &HLC,
    ) -> Result<Option<models::tree::TreeEntry>> {
        // Parse path into components
        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if components.is_empty() {
            // Root path - no tree entry for root itself
            return Ok(None);
        }

        // Start from root tree
        let mut current_tree_id = *root_tree_id;

        // Navigate through each path component
        for (idx, component) in components.iter().enumerate() {
            // Get entries from current tree
            let entries = self
                .storage
                .trees()
                .iter_tree(
                    RepoScope::new(&self.tenant_id, &self.repo_id),
                    &current_tree_id,
                    None,
                    10000,
                )
                .await?;

            // Find entry matching this component
            let matching_entry = if idx == 0 {
                // Root level: need to check node from NODES CF to match by name
                let mut found_entry = None;
                for entry in &entries {
                    if let Some(node) = self
                        .storage
                        .nodes()
                        .get(self.scope(), &entry.node_id, Some(revision))
                        .await?
                    {
                        if node.name == *component {
                            found_entry = Some(entry.clone());
                            break;
                        }
                    }
                }
                found_entry
            } else {
                // Child level: entry_key = node_name
                entries.into_iter().find(|e| e.entry_key == *component)
            };

            match matching_entry {
                Some(entry) => {
                    // If this is the last component, return the entry itself
                    if idx == components.len() - 1 {
                        return Ok(Some(entry));
                    }

                    // Otherwise, navigate to its children tree
                    if let Some(children_tree_id) = entry.children_tree_id {
                        current_tree_id = children_tree_id;
                    } else {
                        // Node exists but has no children - can't navigate further
                        return Ok(None);
                    }
                }
                None => {
                    // Path component not found
                    return Ok(None);
                }
            }
        }

        Ok(None)
    }
}
