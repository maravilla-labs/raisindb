//! Compound index building job handler
//!
//! This module handles background compound index building operations
//! for rebuilding indexes when NodeType definitions change.

use crate::{cf, cf_handle, keys};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::CompoundColumnValue;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

use crate::repositories::{BranchRepositoryImpl, NodeTypeRepositoryImpl, RevisionRepositoryImpl};

/// Handler for compound index building jobs
///
/// This handler processes CompoundIndexBuild jobs by:
/// 1. Extracting parameters from JobType
/// 2. Loading the NodeType definition to get index configuration
/// 3. Scanning all nodes of the specified node_type
/// 4. For each node, extracting column values and indexing them
pub struct CompoundIndexJobHandler {
    db: Arc<DB>,
    node_type_repo: NodeTypeRepositoryImpl,
}

impl CompoundIndexJobHandler {
    /// Create a new compound index job handler
    ///
    /// # Arguments
    ///
    /// * `db` - RocksDB instance for all operations
    /// * `revision_repo` - Revision repository for NodeType lookups
    /// * `branch_repo` - Branch repository for NodeType lookups
    pub fn new(
        db: Arc<DB>,
        revision_repo: Arc<RevisionRepositoryImpl>,
        branch_repo: Arc<BranchRepositoryImpl>,
    ) -> Self {
        Self {
            node_type_repo: NodeTypeRepositoryImpl::new(db.clone(), revision_repo, branch_repo),
            db,
        }
    }

    /// Handle compound index build job
    ///
    /// Processes a CompoundIndexBuild job variant which builds the specified
    /// compound index for all nodes of the given node_type.
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing the JobType::CompoundIndexBuild variant
    /// * `_context` - Job context with tenant, repo, branch, workspace info
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Job type is not CompoundIndexBuild
    /// - NodeType doesn't exist or doesn't have the specified index
    /// - Index building fails
    pub async fn handle(&self, job: &JobInfo, _context: &JobContext) -> Result<()> {
        // Extract parameters from JobType
        let (tenant_id, repo_id, branch, workspace, node_type_name, index_name) =
            match &job.job_type {
                JobType::CompoundIndexBuild {
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    node_type_name,
                    index_name,
                } => (
                    tenant_id.as_str(),
                    repo_id.as_str(),
                    branch.as_str(),
                    workspace.as_str(),
                    node_type_name.as_str(),
                    index_name.as_str(),
                ),
                _ => {
                    return Err(Error::Validation(
                        "Expected CompoundIndexBuild job type".to_string(),
                    ))
                }
            };

        tracing::info!(
            job_id = %job.id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            workspace = %workspace,
            node_type = %node_type_name,
            index_name = %index_name,
            "Processing compound index build job"
        );

        // Load NodeType definition
        use raisin_storage::NodeTypeRepository;
        let node_type = self
            .node_type_repo
            .get(
                raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                node_type_name,
                None,
            )
            .await?
            .ok_or_else(|| Error::NotFound(format!("NodeType '{}' not found", node_type_name)))?;

        // Find the compound index definition
        let compound_indexes = node_type.compound_indexes.as_ref().ok_or_else(|| {
            Error::NotFound(format!(
                "NodeType '{}' has no compound indexes",
                node_type_name
            ))
        })?;

        let index_def = compound_indexes
            .iter()
            .find(|idx| idx.name == index_name)
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "Compound index '{}' not found in NodeType '{}'",
                    index_name, node_type_name
                ))
            })?;

        tracing::debug!(
            job_id = %job.id,
            index_columns = index_def.columns.len(),
            has_order_column = index_def.has_order_column,
            "Loaded compound index definition"
        );

        // Scan all nodes of this type using direct DB access
        let nodes =
            self.scan_nodes_by_type(tenant_id, repo_id, branch, workspace, node_type_name)?;

        tracing::info!(
            job_id = %job.id,
            total_nodes = nodes.len(),
            "Scanned nodes to index"
        );

        // Build index entries in batches for performance
        let batch_size = 1000;
        let mut indexed_count = 0;
        let mut skipped_count = 0;

        for chunk in nodes.chunks(batch_size) {
            let mut batch = WriteBatch::default();
            let cf_compound = cf_handle(&self.db, cf::COMPOUND_INDEX)?;

            // Get the latest revision for this batch operation
            let revision = HLC::now();

            for node in chunk {
                // Extract column values from the node
                let mut column_values = Vec::with_capacity(index_def.columns.len());

                for column_def in &index_def.columns {
                    match Self::extract_compound_column_value(
                        node,
                        &column_def.property,
                        &column_def.column_type,
                    ) {
                        Some(value) => column_values.push(value),
                        None => {
                            // Skip this node if any required column is missing
                            tracing::trace!(
                                job_id = %job.id,
                                node_id = %node.id,
                                missing_property = %column_def.property,
                                "Skipping node: missing column value"
                            );
                            skipped_count += 1;
                            break;
                        }
                    }
                }

                // Only index if we got all required columns
                if column_values.len() == index_def.columns.len() {
                    let is_published = node.published_at.is_some();
                    let key = keys::compound_index_key_versioned(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        index_name,
                        &column_values,
                        &revision,
                        &node.id,
                        is_published,
                    );

                    batch.put_cf(cf_compound, key, b"");
                    indexed_count += 1;
                }
            }

            // Write the batch
            self.db
                .write(batch)
                .map_err(|e| Error::storage(e.to_string()))?;

            tracing::debug!(
                job_id = %job.id,
                indexed = indexed_count,
                skipped = skipped_count,
                "Batch indexed"
            );
        }

        tracing::info!(
            job_id = %job.id,
            total_nodes = nodes.len(),
            indexed_count = indexed_count,
            skipped_count = skipped_count,
            "Compound index build completed"
        );

        Ok(())
    }

    /// Scan nodes of a specific type using direct DB access
    ///
    /// Scans the NODES column family and filters by node_type.
    fn scan_nodes_by_type(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_type_name: &str,
    ) -> Result<Vec<Node>> {
        // Scan all nodes in this workspace and filter by type
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .build_prefix();

        let cf_nodes = cf_handle(&self.db, cf::NODES)?;

        let iter = self.db.iterator_cf(
            cf_nodes,
            rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
        );

        let mut nodes = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        for item in iter {
            let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix) {
                break;
            }

            // Skip tombstone markers (empty value)
            if value.is_empty() {
                continue;
            }

            // Deserialize node
            let node: Node = match rmp_serde::from_slice(&value) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!("Failed to deserialize node: {}", e);
                    continue;
                }
            };

            // Filter by node_type
            if node.node_type != node_type_name {
                continue;
            }

            // Deduplicate by node_id (we only want the latest version)
            if seen_ids.contains(&node.id) {
                continue;
            }
            seen_ids.insert(node.id.clone());

            nodes.push(node);
        }

        Ok(nodes)
    }

    /// Extract a compound column value from a node based on the property name
    ///
    /// Handles both system properties (like __node_type, __created_at) and regular properties.
    ///
    /// # Arguments
    /// * `node` - The node to extract from
    /// * `property` - Property name (e.g., "category", "__node_type", "__created_at")
    /// * `column_type` - Expected column type for proper encoding
    ///
    /// # Returns
    /// Some(CompoundColumnValue) if the property exists, None otherwise
    fn extract_compound_column_value(
        node: &Node,
        property: &str,
        column_type: &raisin_models::nodes::properties::schema::CompoundColumnType,
    ) -> Option<CompoundColumnValue> {
        use raisin_models::nodes::properties::schema::CompoundColumnType;
        use raisin_models::nodes::properties::PropertyValue;

        match property {
            // System property: node_type
            "__node_type" => Some(CompoundColumnValue::String(node.node_type.clone())),

            // System property: created_at
            "__created_at" => node.created_at.map(|dt| {
                let timestamp_micros = dt.timestamp_micros();
                match column_type {
                    CompoundColumnType::Timestamp => {
                        CompoundColumnValue::TimestampDesc(timestamp_micros)
                    }
                    _ => CompoundColumnValue::TimestampAsc(timestamp_micros),
                }
            }),

            // System property: updated_at
            "__updated_at" => node.updated_at.map(|dt| {
                let timestamp_micros = dt.timestamp_micros();
                match column_type {
                    CompoundColumnType::Timestamp => {
                        CompoundColumnValue::TimestampDesc(timestamp_micros)
                    }
                    _ => CompoundColumnValue::TimestampAsc(timestamp_micros),
                }
            }),

            // Regular property from properties map
            prop_name => {
                let prop_value = node.properties.get(prop_name)?;

                // Convert PropertyValue to CompoundColumnValue based on column_type
                match (column_type, prop_value) {
                    (CompoundColumnType::String, PropertyValue::String(s)) => {
                        Some(CompoundColumnValue::String(s.clone()))
                    }
                    (CompoundColumnType::Integer, PropertyValue::Integer(i)) => {
                        Some(CompoundColumnValue::Integer(*i))
                    }
                    (CompoundColumnType::Boolean, PropertyValue::Boolean(b)) => {
                        Some(CompoundColumnValue::Boolean(*b))
                    }
                    _ => {
                        // Type mismatch or unsupported conversion
                        tracing::warn!(
                            "Type mismatch for compound index column '{}': expected {:?}, got {:?}",
                            prop_name,
                            column_type,
                            prop_value
                        );
                        None
                    }
                }
            }
        }
    }
}
