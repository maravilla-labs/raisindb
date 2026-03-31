//! Unique index constraint checking and management

use super::super::super::helpers::hash_property_value;
use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Check unique property constraints before writing a node
    ///
    /// This function validates that all properties marked as `unique: true` in the NodeType
    /// do not conflict with existing nodes in the workspace.
    ///
    /// # Returns
    /// * `Ok(())` - All unique constraints are satisfied
    /// * `Err(Error::Validation)` - A unique constraint violation was detected
    pub(crate) async fn check_unique_constraints(
        &self,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<()> {
        use crate::repositories::UniqueIndexManager;
        use raisin_storage::NodeTypeRepository;

        // Get NodeType to check for unique properties
        let node_type = match self
            .node_type_repo
            .get(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                &node.node_type,
                None,
            )
            .await?
        {
            Some(nt) => nt,
            None => return Ok(()), // No NodeType = no unique constraints
        };

        // Get properties that have unique: true
        let unique_properties = extract_unique_property_names(&node_type);
        if unique_properties.is_empty() {
            return Ok(());
        }

        // Create UniqueIndexManager for checking
        let unique_manager = UniqueIndexManager::new(self.db.clone());

        // Check each unique property
        for prop_name in unique_properties {
            if let Some(prop_value) = node.properties.get(&prop_name) {
                let value_hash = hash_property_value(prop_value);

                if let Some(conflicting_node_id) = unique_manager.check_unique_conflict(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node.node_type,
                    &prop_name,
                    &value_hash,
                    &node.id,
                )? {
                    return Err(raisin_error::Error::Validation(format!(
                        "Property '{}' must be unique. Value '{}' is already used by node '{}'",
                        prop_name, value_hash, conflicting_node_id
                    )));
                }
            }
        }

        Ok(())
    }

    /// Add unique index entries for a node to a WriteBatch
    ///
    /// This function writes unique index entries for all properties marked as `unique: true`
    /// in the NodeType. The indexes enable O(1) conflict detection.
    pub(crate) async fn add_unique_indexes_to_batch(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        use crate::repositories::UniqueIndexManager;
        use raisin_storage::NodeTypeRepository;

        // Get NodeType to check for unique properties
        let node_type = match self
            .node_type_repo
            .get(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                &node.node_type,
                None,
            )
            .await?
        {
            Some(nt) => nt,
            None => return Ok(()), // No NodeType = no unique indexes
        };

        // Get properties that have unique: true
        let unique_properties = extract_unique_property_names(&node_type);
        if unique_properties.is_empty() {
            return Ok(());
        }

        // Create UniqueIndexManager for writing
        let unique_manager = UniqueIndexManager::new(self.db.clone());

        // Add index entries for each unique property with a value
        for prop_name in unique_properties {
            if let Some(prop_value) = node.properties.get(&prop_name) {
                let value_hash = hash_property_value(prop_value);

                unique_manager.add_unique_index_to_batch(
                    batch,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node.node_type,
                    &prop_name,
                    &value_hash,
                    revision,
                    &node.id,
                )?;

                tracing::trace!(
                    "Added unique index for node '{}' property '{}' value '{}'",
                    node.id,
                    prop_name,
                    value_hash
                );
            }
        }

        Ok(())
    }

    /// Add tombstones for unique index entries when a node is deleted or unique property value changes
    ///
    /// This function writes tombstones for unique index entries that are no longer valid.
    /// Call this when:
    /// - Deleting a node with unique properties
    /// - Updating a node where a unique property value has changed
    pub(crate) async fn add_unique_tombstones_to_batch(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        use crate::repositories::UniqueIndexManager;
        use raisin_storage::NodeTypeRepository;

        // Get NodeType to check for unique properties
        let node_type = match self
            .node_type_repo
            .get(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                &node.node_type,
                None,
            )
            .await?
        {
            Some(nt) => nt,
            None => return Ok(()), // No NodeType = no unique indexes to tombstone
        };

        // Get properties that have unique: true
        let unique_properties = extract_unique_property_names(&node_type);
        if unique_properties.is_empty() {
            return Ok(());
        }

        // Create UniqueIndexManager for writing tombstones
        let unique_manager = UniqueIndexManager::new(self.db.clone());

        // Add tombstones for each unique property with a value
        for prop_name in unique_properties {
            if let Some(prop_value) = node.properties.get(&prop_name) {
                let value_hash = hash_property_value(prop_value);

                unique_manager.add_unique_tombstone_to_batch(
                    batch,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node.node_type,
                    &prop_name,
                    &value_hash,
                    revision,
                )?;

                tracing::trace!(
                    "Added unique index tombstone for node '{}' property '{}' value '{}'",
                    node.id,
                    prop_name,
                    value_hash
                );
            }
        }

        Ok(())
    }
}

/// Extract property names that have `unique: true` from a NodeType
fn extract_unique_property_names(node_type: &raisin_models::nodes::NodeType) -> Vec<String> {
    match node_type.properties {
        Some(ref props) => props
            .iter()
            .filter_map(|p| {
                if p.unique.unwrap_or(false) {
                    p.name.clone()
                } else {
                    None
                }
            })
            .collect::<Vec<String>>(),
        None => Vec::new(),
    }
}
