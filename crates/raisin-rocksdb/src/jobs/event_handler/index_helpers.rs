//! Index-related helper methods for the event handler
//!
//! Provides functionality to check embedding configuration and determine
//! index settings (fulltext vs vector) for nodes based on their NodeType.

use super::UnifiedJobEventHandler;
use raisin_embeddings::storage::TenantEmbeddingConfigStore;
use raisin_error::Result;
use raisin_storage::{NodeRepository, NodeTypeRepository, Storage, StorageScope};

/// Index settings for a node (fulltext and vector indexability)
///
/// Used to determine which index types should be applied to a node,
/// fetching node and node_type data only once.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct IndexSettings {
    /// Whether fulltext indexing should be performed
    pub fulltext: bool,
    /// Whether vector embedding should be performed
    pub vector: bool,
}

impl UnifiedJobEventHandler {
    /// Check if embeddings are enabled for a tenant
    pub(crate) async fn embeddings_enabled(&self, tenant_id: &str) -> Result<bool> {
        match self
            .storage
            .tenant_embedding_config_repository()
            .get_config(tenant_id)
        {
            Ok(Some(config)) => Ok(config.enabled),
            Ok(None) => {
                tracing::debug!(
                    tenant_id = %tenant_id,
                    "No embedding config found for tenant, treating as disabled"
                );
                Ok(false)
            }
            Err(e) => {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    error = %e,
                    "Failed to check embedding config, treating as disabled"
                );
                Ok(false)
            }
        }
    }

    /// Get index settings for a node, using cached node data if available
    ///
    /// This consolidates what was previously 4 DB reads (2 for fulltext, 2 for vector)
    /// into at most 2 DB reads (1 for node, 1 for node_type), or just 1 if node_data
    /// is provided from event metadata.
    ///
    /// Returns `IndexSettings` with both fulltext and vector flags set based on:
    /// - NodeType.indexable (defaults to true if unset)
    /// - NodeType.index_types (defaults to all types if unset)
    ///
    /// # Arguments
    /// * `node_data` - Optional node data from event metadata (avoids node fetch)
    pub(crate) async fn get_index_settings(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        node_data: Option<&raisin_models::nodes::Node>,
    ) -> IndexSettings {
        use raisin_models::nodes::properties::schema::IndexType;

        // Use provided node_data if available, otherwise fetch from DB
        let node: std::borrow::Cow<'_, raisin_models::nodes::Node> = if let Some(n) = node_data {
            tracing::trace!(
                node_id = %node_id,
                "Using node_data from event metadata for index check (skipped DB read)"
            );
            std::borrow::Cow::Borrowed(n)
        } else {
            // Fetch the node to get its type (1 DB read)
            match self
                .storage
                .nodes()
                .get(
                    StorageScope::new(tenant_id, repo_id, branch, workspace_id),
                    node_id,
                    None,
                )
                .await
            {
                Ok(Some(n)) => std::borrow::Cow::Owned(n),
                Ok(None) => {
                    tracing::debug!(
                        node_id = %node_id,
                        "Node not found, skipping index check"
                    );
                    return IndexSettings::default();
                }
                Err(e) => {
                    tracing::warn!(
                        node_id = %node_id,
                        error = %e,
                        "Failed to fetch node for index check, treating as non-indexable"
                    );
                    return IndexSettings::default();
                }
            }
        };

        // Fetch the NodeType definition (1 DB read)
        let node_type_def = match self
            .storage
            .node_types()
            .get(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                &node.node_type,
                None,
            )
            .await
        {
            Ok(Some(nt)) => nt,
            Ok(None) => {
                tracing::debug!(
                    node_type = %node.node_type,
                    "NodeType not found, treating as indexable (default)"
                );
                return IndexSettings {
                    fulltext: true,
                    vector: true,
                };
            }
            Err(e) => {
                tracing::warn!(
                    node_type = %node.node_type,
                    error = %e,
                    "Failed to fetch NodeType, treating as indexable (default)"
                );
                return IndexSettings {
                    fulltext: true,
                    vector: true,
                };
            }
        };

        // Check if indexable is explicitly set to false
        if node_type_def.indexable == Some(false) {
            tracing::debug!(
                node_type = %node.node_type,
                "NodeType has indexable=false, skipping all indexes"
            );
            return IndexSettings::default();
        }

        // Check index_types to determine which indexes are allowed
        let (fulltext, vector) = if let Some(index_types) = &node_type_def.index_types {
            (
                index_types.contains(&IndexType::Fulltext),
                index_types.contains(&IndexType::Vector),
            )
        } else {
            // Default: all index types allowed
            (true, true)
        };

        tracing::trace!(
            node_id = %node_id,
            node_type = %node.node_type,
            fulltext = %fulltext,
            vector = %vector,
            "Determined index settings for node"
        );

        IndexSettings { fulltext, vector }
    }
}
