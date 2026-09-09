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

        // Compound indexes INCLUDING the ones inherited through `extends`.
        //
        // This used to read `node_type.compound_indexes` off the raw stored
        // record, which never contains an ancestor's declarations — so a subtype
        // wrote NO entries for an index declared on its parent, and every query
        // on that subtype silently fell back to a scan while the parent's own
        // queries were fast. Inheritance merges `compound_indexes` along the
        // chain, and the write path has to honour that or the index is a lie for
        // half the family.
        //
        // Resolved locally rather than through raisin-core's resolver because
        // the dependency points the other way; the chain is shallow and the
        // NodeType reads are cached.
        let inherited = self
            .resolve_inherited_compound_indexes(&node_type, tenant_id, repo_id, branch)
            .await?;
        let compound_indexes = match inherited {
            ref indexes if !indexes.is_empty() => indexes,
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
    /// ONE implementation, shared with the rebuild job.
    ///
    /// The rebuild carried its own copy, which silently diverged the moment a
    /// column was added here: a rebuild would then write entries WITHOUT that
    /// column, leaving an index the planner matches and the data does not
    /// support. Associated fn on the impl only for call-site convenience — it
    /// reads nothing from `self`.
    pub(crate) fn extract_compound_column_value(
        node: &Node,
        property: &str,
        column_type: &raisin_models::nodes::properties::schema::CompoundColumnType,
    ) -> Option<raisin_storage::CompoundColumnValue> {
        use raisin_models::nodes::properties::schema::CompoundColumnType;
        use raisin_storage::CompoundColumnValue;

        match property {
            // System property: node_type
            "__node_type" => Some(CompoundColumnValue::String(node.node_type.clone())),

            // System property: the containing directory.
            //
            // This is what makes `CHILD_OF(p) ORDER BY <col>` a seek: hierarchy
            // enters the index as an EQUALITY on the leading column, which is the
            // only shape a sorted index can combine with a trailing order column.
            // (A subtree is a path RANGE, and a range cannot precede an order
            // column — that needs a materialised ancestor column instead.)
            //
            // Derived from `node.path`, not `node.parent`: the planner sees
            // `CHILD_OF('/a/b')` as a PATH and cannot resolve it to a parent id
            // without an async lookup. The trade is that a subtree move rewrites
            // descendants' entries — but `move_node_tree_impl` already rewrites
            // PATH_INDEX and NODE_PATH per descendant, so this rides along with
            // work that is already O(subtree).
            // Uses the canonical `Node::parent_path()` rather than re-deriving
            // it: the planner normalises `CHILD_OF(p)` to the same string, and
            // if the two sides ever disagreed the index would simply never match
            // — correct results, permanently unused index, no error anywhere.
            // Root nodes yield `None` and are not indexed under this column,
            // which is right: nothing is CHILD_OF a node's own root.
            "__parent_path" => node.parent_path().map(CompoundColumnValue::String),

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

    /// A NodeType's compound indexes, merged along its `extends` chain.
    ///
    /// Most-derived wins on a name collision, matching the core resolver: parent
    /// first, then own. A missing or cyclic parent simply ends the walk — an
    /// index write must not fail because one NodeType is malformed.
    pub(crate) async fn resolve_inherited_compound_indexes(
        &self,
        node_type: &raisin_models::nodes::types::node_type::NodeType,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Vec<CompoundIndexDefinition>> {
        use raisin_storage::NodeTypeRepository;

        const MAX_DEPTH: usize = 20;

        // Walk up to the root, collecting each level's declarations.
        let mut chain: Vec<Vec<CompoundIndexDefinition>> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut current = Some(node_type.clone());
        let mut depth = 0usize;

        while let Some(nt) = current {
            if depth >= MAX_DEPTH || !seen.insert(nt.name.clone()) {
                break;
            }
            depth += 1;
            chain.push(nt.compound_indexes.clone().unwrap_or_default());

            current = match nt.extends.as_deref() {
                Some(parent) if !parent.is_empty() => self
                    .node_type_repo
                    .get(
                        raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
                        parent,
                        None,
                    )
                    .await
                    .ok()
                    .flatten(),
                _ => None,
            };
        }

        // Parent first so a derived declaration of the same NAME replaces it.
        let mut merged: Vec<CompoundIndexDefinition> = Vec::new();
        for level in chain.into_iter().rev() {
            for idx in level {
                if let Some(existing) = merged.iter_mut().find(|e| e.name == idx.name) {
                    *existing = idx;
                } else {
                    merged.push(idx);
                }
            }
        }
        Ok(merged)
    }
}
