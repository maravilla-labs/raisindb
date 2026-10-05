//! Compound index operations (multi-column indexes): the repository's entry
//! points to the one writer (`indexing::compound`), plus the one column
//! extractor.

use super::super::super::NodeRepositoryImpl;
use crate::indexing::compound::{write_compound_delta, DefsSet};
use crate::indexing::{Baseline, IndexCtx};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// The schema-driven index definitions of `types` on the branch
    /// (compound declarations with inheritance, unique property names),
    /// resolved through the shared cache — async, so call it BEFORE any batch
    /// lock is taken.
    pub(crate) async fn index_defs(&self, ctx: &IndexCtx<'_>, types: &[&str]) -> Result<DefsSet> {
        DefsSet::resolve(
            &self.db,
            self.node_type_repo.as_ref(),
            raisin_storage::BranchScope::new(ctx.tenant_id, ctx.repo_id, ctx.branch),
            types,
        )
        .await
    }

    /// Stage `new`'s COMPOUND_INDEX write against `baseline` (the property
    /// index's baseline for the same write — see `indexing::compound::writer`).
    pub(crate) async fn add_compound_delta_to_batch(
        &self,
        batch: &mut WriteBatch,
        ctx: &IndexCtx<'_>,
        baseline: Baseline<'_>,
        new: &Node,
        revision: &HLC,
    ) -> Result<()> {
        let types = crate::indexing::compound::types_of(&baseline, new);
        let defs = self.index_defs(ctx, &types).await?;
        if !defs.any_compound() {
            return Ok(());
        }
        write_compound_delta(batch, &self.db, ctx, &defs, baseline, new, revision)?;
        Ok(())
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
                    // One text encoding for both spellings a stored string
                    // can come back as (`CompoundColumnValue::text`).
                    (CompoundColumnType::String, PropertyValue::String(s)) => {
                        Some(CompoundColumnValue::text(s))
                    }
                    (CompoundColumnType::String, PropertyValue::Date(d)) => {
                        Some(CompoundColumnValue::date_text(**d))
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
