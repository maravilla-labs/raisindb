//! Compound index operations (multi-column indexes)

use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};

impl NodeRepositoryImpl {
    /// Add compound indexes for a node based on its NodeType's compound_indexes configuration
    ///
    /// This reads the NodeType definition to find any compound indexes defined for this node type,
    /// then extracts the required column values from the node and indexes them.
    pub(crate) async fn add_compound_indexes_to_batch(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        use raisin_storage::NodeTypeRepository;

        // Get NodeType to check for compound indexes
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
            None => return Ok(()), // No NodeType = no compound indexes
        };

        // Check if this NodeType has compound indexes
        let compound_indexes = match node_type.compound_indexes {
            Some(ref indexes) if !indexes.is_empty() => indexes,
            _ => return Ok(()), // No compound indexes defined
        };

        Self::write_compound_entries_to_batch(
            &self.db,
            batch,
            compound_indexes,
            node,
            tenant_id,
            repo_id,
            branch,
            workspace,
            revision,
        )
    }

    /// Write compound-index entries for a node into a WriteBatch (synchronous).
    ///
    /// This is the SINGLE source of compound-index write encoding, shared by the
    /// repository create/update paths, the transaction (SQL DML) path, and the
    /// compound-index rebuild. Callers must resolve the NodeType's
    /// `compound_indexes` first (an async NodeType fetch) and pass it in, keeping
    /// this body lock-friendly and `await`-free so it can run while a WriteBatch
    /// mutex guard is held.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_compound_entries_to_batch(
        db: &DB,
        batch: &mut WriteBatch,
        compound_indexes: &[CompoundIndexDefinition],
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf_compound = cf_handle(db, cf::COMPOUND_INDEX)?;
        let is_published = node.published_at.is_some();

        // Process each compound index
        for index_def in compound_indexes {
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
                        // Skip this index if any required column is missing
                        tracing::debug!(
                            "Skipping compound index '{}' for node '{}': missing property '{}'",
                            index_def.name,
                            node.id,
                            column_def.property
                        );
                        break;
                    }
                }
            }

            // Only index if we got all required columns
            if column_values.len() == index_def.columns.len() {
                let key = keys::compound_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &index_def.name,
                    &column_values,
                    revision,
                    &node.id,
                    is_published,
                );

                batch.put_cf(cf_compound, key, b"");

                tracing::trace!(
                    "Indexed node '{}' in compound index '{}' with {} columns",
                    node.id,
                    index_def.name,
                    column_values.len()
                );
            }
        }

        Ok(())
    }

    /// Tombstone a node's existing compound-index entries (for UPDATE).
    ///
    /// On an update the column values may have changed; the old-value entries
    /// must be tombstoned before the new ones are written so a scan keyed on the
    /// OLD value no longer returns the node. Reuses the shared tombstone logic.
    pub(crate) fn add_compound_tombstones_to_batch(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<()> {
        use crate::tombstones::{
            tombstone_compound_indexes_only, TombstoneColumnFamilies, TombstoneContext,
        };
        let ctx = TombstoneContext::new(tenant_id, repo_id, branch, workspace);
        let cfs = TombstoneColumnFamilies::from_arc_db(&self.db)?;
        tombstone_compound_indexes_only(batch, self.db.as_ref(), &ctx, &cfs, node)
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
    ) -> Option<raisin_storage::CompoundColumnValue> {
        use raisin_models::nodes::properties::schema::CompoundColumnType;
        use raisin_storage::CompoundColumnValue;

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
