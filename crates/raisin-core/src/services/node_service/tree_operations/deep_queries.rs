//! Deep tree query operations.
//!
//! Provides methods for querying nested, flat, and array-format
//! representations of deep tree structures.

use raisin_error::Result;
use raisin_models as models;
use raisin_storage::{NodeRepository, Storage};

use super::super::NodeService;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage> NodeService<S> {
    /// Gets nested tree structure with depth limit
    pub async fn deep_children_nested(
        &self,
        parent_path: &str,
        max_depth: u32,
    ) -> Result<std::collections::HashMap<String, models::nodes::DeepNode>> {
        // Validate parent exists (except for root)
        if parent_path != "/" && !parent_path.is_empty() {
            let parent_exists = self
                .storage
                .nodes()
                .get_by_path(self.scope(), parent_path, self.revision.as_ref())
                .await?
                .is_some();
            if !parent_exists {
                return Err(raisin_error::Error::NotFound(format!(
                    "Parent path not found: {}",
                    parent_path
                )));
            }
        }
        self.storage
            .nodes()
            .deep_children_nested(self.scope(), parent_path, max_depth, self.revision.as_ref())
            .await
    }

    /// Gets flat Vec of all descendants with depth limit (fractional index order)
    pub async fn deep_children_flat(
        &self,
        parent_path: &str,
        max_depth: u32,
    ) -> Result<Vec<models::nodes::Node>> {
        // Validate parent exists (except for root)
        if parent_path != "/" && !parent_path.is_empty() {
            let parent_exists = self
                .storage
                .nodes()
                .get_by_path(self.scope(), parent_path, self.revision.as_ref())
                .await?
                .is_some();
            if !parent_exists {
                return Err(raisin_error::Error::NotFound(format!(
                    "Parent path not found: {}",
                    parent_path
                )));
            }
        }
        self.storage
            .nodes()
            .deep_children_flat(self.scope(), parent_path, max_depth, self.revision.as_ref())
            .await
    }

    /// Gets DX-friendly array format with nested children
    pub async fn deep_children_array(
        &self,
        parent_path: &str,
        max_depth: u32,
    ) -> Result<Vec<models::nodes::NodeWithChildren>> {
        // Validate parent exists (except for root)
        if parent_path != "/" && !parent_path.is_empty() {
            let parent_exists = self
                .storage
                .nodes()
                .get_by_path(self.scope(), parent_path, self.revision.as_ref())
                .await?
                .is_some();
            if !parent_exists {
                return Err(raisin_error::Error::NotFound(format!(
                    "Parent path not found: {}",
                    parent_path
                )));
            }
        }
        self.storage
            .nodes()
            .deep_children_array(self.scope(), parent_path, max_depth, self.revision.as_ref())
            .await
    }
}
