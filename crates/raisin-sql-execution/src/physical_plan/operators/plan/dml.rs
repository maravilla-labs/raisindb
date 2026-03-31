//! DML (Data Manipulation Language) operator documentation and helpers.
//!
//! This module documents the DML variants of [`PhysicalPlan`].
//!
//! ## PhysicalInsert
//!
//! Inserts new rows into a schema table (NodeTypes, Archetypes, ElementTypes).
//! Each row is created from column values and stored via the appropriate
//! repository. Also used for UPSERT operations when `is_upsert` is true.
//!
//! ### Fields
//! - `target` - Target table for insertion
//! - `schema` - Table schema definition
//! - `columns` - Column names for insertion (in order)
//! - `values` - Values to insert (outer vec = rows, inner vec = values per row)
//! - `is_upsert` - Whether this is an UPSERT (create-or-update) vs INSERT
//!
//! ## PhysicalUpdate
//!
//! Updates existing rows in a schema table based on a WHERE clause. Fetches
//! matching rows, applies assignments, and stores updated versions.
//!
//! ### Fields
//! - `target` - Target table for update
//! - `schema` - Table schema definition
//! - `assignments` - Column assignments (column_name, new_value_expr)
//! - `filter` - Filter to identify rows to update (WHERE clause)
//! - `branch_override` - Optional branch override for cross-branch operations
//!
//! ## PhysicalDelete
//!
//! Deletes rows from a schema table based on a WHERE clause. Extracts
//! identifiers from the filter and calls repository delete methods.
//!
//! ### Fields
//! - `target` - Target table for deletion
//! - `schema` - Table schema definition
//! - `filter` - Filter to identify rows to delete (WHERE clause)
//! - `branch_override` - Optional branch override for cross-branch operations
//!
//! ## PhysicalOrder
//!
//! Reorders a node relative to a sibling in the tree structure. Maps to
//! `NodeService::move_child_before` or `move_child_after`.
//!
//! ### Fields
//! - `source` - Source node reference (node to move)
//! - `target` - Target node reference (reference sibling)
//! - `position` - Position relative to target (Above = before, Below = after)
//! - `workspace` - Workspace containing the nodes
//! - `branch_override` - Optional branch override
//!
//! ## PhysicalMove
//!
//! Moves a node and all its descendants to a new parent in the tree structure.
//! Similar to a file system move operation.
//!
//! For trees with > 5000 descendants, the operation is executed asynchronously
//! and returns a `job_id` for tracking. Otherwise, it executes synchronously.
//!
//! ### Fields
//! - `source` - Source node reference (node to move)
//! - `target_parent` - Target parent node reference (new parent)
//! - `workspace` - Workspace containing the nodes
//! - `branch_override` - Optional branch override
//!
//! ## PhysicalCopy
//!
//! Copies a node (and optionally its descendants) to a new parent location.
//! New node IDs are generated. Publish state is cleared on copied nodes.
//!
//! For trees with > 5000 descendants, the operation is executed asynchronously
//! and returns a `job_id` for tracking. Otherwise, it executes synchronously.
//!
//! ### Fields
//! - `source` - Source node reference (node to copy)
//! - `target_parent` - Target parent (new parent for the copy)
//! - `new_name` - Optional new name for the copied node
//! - `recursive` - Whether to copy recursively (COPY TREE) or single node
//! - `workspace` - Workspace containing the nodes
//! - `branch_override` - Optional branch override
//!
//! ## PhysicalTranslate
//!
//! Updates translations for nodes in a specific locale. Node-level translations
//! use JSON Pointer paths (e.g., "/title", "/metadata/author"). Block-level
//! translations are stored separately keyed by block UUID.
//!
//! ### Fields
//! - `locale` - Target locale code (e.g., "de", "fr", "en-US")
//! - `node_translations` - Node-level translations: JSON Pointer -> value
//! - `block_translations` - Block-level translations: block_uuid -> (JSON Pointer -> value)
//! - `filter` - Filter to select nodes to translate
//! - `workspace` - Workspace containing the nodes
//! - `branch_override` - Optional branch override
//!
//! ## PhysicalRelate
//!
//! Creates a directed relationship between two nodes in the graph. Writes to
//! the RELATION_INDEX column family.
//!
//! ### Example
//! ```sql
//! RELATE FROM path='/content/page' TO path='/assets/image' TYPE 'references';
//! RELATE FROM path='/page' IN WORKSPACE 'main' TO path='/asset' IN WORKSPACE 'media' WEIGHT 1.5;
//! ```
//!
//! ### Fields
//! - `source` - Source node endpoint (node_ref + workspace)
//! - `target` - Target node endpoint (node_ref + workspace)
//! - `relation_type` - Relationship type (e.g., "references", "tagged")
//! - `weight` - Optional weight for graph algorithms
//! - `branch_override` - Optional branch override
//!
//! ## PhysicalUnrelate
//!
//! Removes a directed relationship between two nodes in the graph.
//! Deletes from the RELATION_INDEX column family.
//!
//! ### Example
//! ```sql
//! UNRELATE FROM path='/content/page' TO path='/assets/image';
//! UNRELATE FROM path='/page' TO path='/asset' TYPE 'tagged';
//! ```
//!
//! ### Fields
//! - `source` - Source node endpoint (node_ref + workspace)
//! - `target` - Target node endpoint (node_ref + workspace)
//! - `relation_type` - Optional type filter (if specified, only removes this type)
//! - `branch_override` - Optional branch override
//!
//! ## PhysicalRestore
//!
//! Restores a node (and optionally its descendants) to its state at a previous
//! revision. The node stays at its current path -- this is an in-place restore,
//! not a copy.
//!
//! ### Example
//! ```sql
//! RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2
//! RESTORE TREE NODE path='/products/category' TO REVISION HEAD~5
//! RESTORE NODE id='uuid' TO REVISION HEAD~2 TRANSLATIONS ('en', 'de')
//! ```
//!
//! ### Fields
//! - `node` - Node reference to restore (by path or id)
//! - `revision` - Revision reference to restore from
//! - `recursive` - Whether to restore children (RESTORE TREE NODE)
//! - `translations` - Specific translations to restore (None = all)
//! - `branch_override` - Optional branch override

use super::PhysicalPlan;

impl PhysicalPlan {
    /// Returns true if this is a DML (data manipulation) operator.
    pub fn is_dml(&self) -> bool {
        matches!(
            self,
            PhysicalPlan::PhysicalInsert { .. }
                | PhysicalPlan::PhysicalUpdate { .. }
                | PhysicalPlan::PhysicalDelete { .. }
                | PhysicalPlan::PhysicalOrder { .. }
                | PhysicalPlan::PhysicalMove { .. }
                | PhysicalPlan::PhysicalCopy { .. }
                | PhysicalPlan::PhysicalTranslate { .. }
                | PhysicalPlan::PhysicalRelate { .. }
                | PhysicalPlan::PhysicalUnrelate { .. }
                | PhysicalPlan::PhysicalRestore { .. }
        )
    }
}
