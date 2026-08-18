// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Node repository trait definitions.
//!
//! This module contains the `NodeRepository` trait which provides CRUD operations,
//! tree management, property access, and publishing workflows for nodes within workspaces.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_models::tree::ChangeOperation;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::node_operations::{
    CreateNodeOptions, DeleteNodeOptions, ListOptions, NodeWithPopulatedChildren, UpdateNodeOptions,
};
use crate::scope::{BranchScope, StorageScope};

/// A single node touched by a cross-branch node-set copy.
///
/// Carries enough context (id, path, type, operation) for callers to emit
/// per-node change notifications for the target branch without re-reading
/// the affected nodes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CrossBranchNodeChange {
    /// Id of the node (ids are preserved across branches)
    pub node_id: String,
    /// Path of the node on the target branch (for deletions: the pruned path)
    pub path: String,
    /// NodeType of the node
    pub node_type: String,
    /// What happened on the target branch (Added / Modified / Deleted)
    pub operation: ChangeOperation,
}

/// One entry of a parent's editorial (fractional-index) child order.
///
/// Returned by [`NodeRepository::list_ordered_children_page`]. The
/// `order_label` is opaque but lexicographically sortable, and is directly
/// usable as a keyset cursor for the next page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrderedChild {
    /// Id of the child node.
    pub child_id: String,
    /// Opaque, lexicographically sortable editorial order label.
    ///
    /// Treat as a cursor token: pass the last row's label back as
    /// `after_label` to fetch the next page. Do not parse or construct it.
    pub order_label: String,
    /// Name of the child (carried in the index entry, so no node load needed).
    pub name: String,
}

/// Summary of a cross-branch node-set copy (see
/// [`NodeRepository::copy_nodes_across_branches`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CrossBranchCopySummary {
    /// Number of nodes written to the target branch (added + modified)
    pub copied: usize,
    /// Number of target-branch nodes pruned (`delete_missing` only)
    pub deleted: usize,
    /// The single revision all changes were committed under
    pub revision: HLC,
    /// Per-node change list (drives change notifications / audit)
    pub changes: Vec<CrossBranchNodeChange>,
}

/// Repository interface for node storage operations.
///
/// Provides CRUD operations, tree management, property access, and publishing
/// workflows for nodes within workspaces.
///
/// # Changes in Version 2.0
///
/// - **Separated create/update**: `create()` and `update()` replace `put()`/`add()`
/// - **Schema validation**: All create/update operations validate against NodeType schemas
/// - **Performance controls**: List methods take `ListOptions` for has_children computation
/// - **Explicit semantics**: Methods clearly indicate their behavior and constraints
///
/// # Scoped Architecture
///
/// All methods take a `StorageScope` (or `BranchScope`) parameter that bundles:
/// - Multi-tenant isolation (`tenant_id`)
/// - Repository (project/database) scoping (`repo_id`)
/// - Git-like branch operations (`branch`)
/// - Workspace scoping (`workspace`)
///
/// # Translation Handling
///
/// Translations are NOT handled in this repository. Use:
/// - `TranslationService` for CRUD operations on translations
/// - `TranslationResolver` for applying translations to nodes
/// - Node deletion automatically cascades to translations
pub trait NodeRepository: Send + Sync {
    // ========================================================================
    // Core CRUD Operations
    // ========================================================================

    /// Get a single node by ID (does NOT compute has_children).
    ///
    /// Use this for:
    /// - Direct node lookups by ID
    /// - SQL query results (where has_children is not needed)
    /// - Internal operations
    ///
    /// # Returns
    /// - `Ok(Some(node))` - Node found, has_children is None
    /// - `Ok(None)` - Node not found
    fn get(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::Node>>> + Send;

    /// Get a node with its direct children populated.
    ///
    /// Use this for:
    /// - API endpoints that need to return children
    /// - UI tree navigation
    /// - Building hierarchical responses
    ///
    /// This method:
    /// - Fetches the node
    /// - Fetches all direct children
    /// - Computes has_children=true/false (since we know children exist or not)
    ///
    /// # Returns
    /// - `Ok(Some(result))` - Node with children populated
    /// - `Ok(None)` - Node not found
    ///
    /// # Performance
    /// This performs 2 queries: 1 for parent + 1 for list_children.
    /// For deep trees, use `deep_children_*` methods instead.
    fn get_with_children(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<NodeWithPopulatedChildren>>> + Send;

    /// List a node's revision history, newest first (git-log style).
    ///
    /// Walks the MVCC revisions of the node and returns a lightweight entry per
    /// revision (`revision`, `updated_at`, `updated_by`, `deleted`). This is
    /// always available regardless of the `auditable` flag — it reflects the
    /// structural version history, not the opt-in audit log.
    ///
    /// Use the returned `revision` with the `at_revision` reads to fetch the
    /// full snapshot of any historical version.
    ///
    /// # Arguments
    /// * `node_id` - Node identifier
    /// * `limit` - Optional cap on the number of revisions returned (newest first)
    fn get_node_history(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        limit: Option<usize>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::NodeRevisionEntry>>> + Send;

    /// Create a new node (fails if node with same ID or path already exists).
    ///
    /// Use this for:
    /// - POST /nodes endpoints
    /// - Creating brand new content
    /// - Import operations where duplicates should error
    ///
    /// # Validation (when enabled in options)
    /// 1. **Schema validation**: Properties match NodeType property schemas
    /// 2. **Required properties**: All required properties present
    /// 3. **Strict mode**: No extra properties if NodeType.strict=true
    /// 4. **Parent-child types**: Parent's allowed_children includes this type
    /// 5. **Workspace types**: Workspace allows this node type
    ///
    /// # Errors
    /// - `Error::Conflict` - Node with ID or path already exists
    /// - `Error::Validation` - Schema validation failed
    /// - `Error::NotFound` - Parent node or NodeType doesn't exist
    fn create(
        &self,
        scope: StorageScope<'_>,
        node: models::nodes::Node,
        options: CreateNodeOptions,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Create a node with automatic parent directory creation (deep create).
    ///
    /// This method creates a node at the specified path, automatically creating
    /// any missing parent directories along the way. All parent folders and the
    /// target node are created atomically in a single WriteBatch with the same revision.
    ///
    /// Use this for:
    /// - Creating nodes in deep hierarchies without manual parent setup
    /// - Import operations where directory structure isn't guaranteed
    /// - API endpoints that should auto-create parent folders
    ///
    /// # Arguments
    /// * `path` - Full path where node should be created (e.g., "/docs/guides/intro")
    /// * `node` - The node to create (path field will be overwritten with `path` parameter)
    /// * `parent_node_type` - NodeType to use for auto-created parent folders (e.g., "raisin:Folder")
    /// * `options` - Creation options (applied to target node, parents use minimal validation)
    ///
    /// # Behavior
    /// 1. Parses path into segments
    /// 2. Creates missing parent folders with `parent_node_type`
    /// 3. Creates target node at final path
    /// 4. All operations use SAME revision for proper MVCC
    /// 5. Atomic commit via WriteBatch
    fn create_deep_node(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        node: models::nodes::Node,
        parent_node_type: &str,
        options: CreateNodeOptions,
    ) -> impl std::future::Future<Output = Result<models::nodes::Node>> + Send;

    /// Update an existing node (fails if node doesn't exist).
    ///
    /// Use this for:
    /// - PUT /nodes/:id endpoints
    /// - PATCH operations to modify properties
    /// - Updating existing content
    ///
    /// # Validation (when enabled in options)
    /// 1. **Existence check**: Node must exist
    /// 2. **Schema validation**: Properties match NodeType property schemas
    /// 3. **Type change guard**: Prevents changing node_type unless allowed
    ///
    /// # Errors
    /// - `Error::NotFound` - Node doesn't exist
    /// - `Error::Validation` - Schema validation failed or type change blocked
    fn update(
        &self,
        scope: StorageScope<'_>,
        node: models::nodes::Node,
        options: UpdateNodeOptions,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Delete a node by ID.
    ///
    /// # Cascade Behavior
    /// - `options.cascade=true` (default): Recursively delete all descendants
    /// - `options.cascade=false`: Fail if node has children (unless check_has_children=false)
    ///
    /// # Errors
    /// - Returns `Ok(false)` if node doesn't exist
    /// - `Error::Validation` - Node has children and cascade=false with check_has_children=true
    ///
    /// # Returns
    /// `true` if node was deleted, `false` if not found
    fn delete(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        options: DeleteNodeOptions,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    // ========================================================================
    // List Operations (with performance controls)
    // ========================================================================

    /// List all nodes of a specific type.
    ///
    /// # Performance
    /// - `options.compute_has_children=false`: Fast, skips child checks
    /// - `options.compute_has_children=true`: Slower, populates has_children for each node
    ///
    /// Use `ListOptions::for_api()` for API responses, `ListOptions::for_sql()` for queries.
    fn list_by_type(
        &self,
        scope: StorageScope<'_>,
        node_type: &str,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send;

    /// List all direct children of a parent.
    ///
    /// # Performance
    /// See `list_by_type` for has_children computation behavior.
    fn list_by_parent(
        &self,
        scope: StorageScope<'_>,
        parent_id: &str,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send;

    /// List all nodes in a workspace.
    ///
    /// # Performance
    /// See `list_by_type` for has_children computation behavior.
    /// For large workspaces (>100k nodes), consider using `count_all()` instead.
    fn list_all(
        &self,
        scope: StorageScope<'_>,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send;

    /// List root-level nodes (nodes with parent="/").
    ///
    /// # Performance
    /// See `list_by_type` for has_children computation behavior.
    fn list_root(
        &self,
        scope: StorageScope<'_>,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send;

    /// List direct children of a parent by path.
    ///
    /// Similar to `list_by_parent` but uses parent path instead of ID.
    fn list_children(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send;

    /// Stream ordered child IDs without loading full node objects.
    ///
    /// This is a low-level streaming primitive for ORDER BY path optimization.
    /// Returns child IDs in their fractional index order (based on order_label).
    /// Memory-efficient: returns IDs only, not full Node objects.
    ///
    /// Use this when you need to traverse the tree without loading all nodes into memory.
    fn stream_ordered_child_ids(
        &self,
        scope: StorageScope<'_>,
        parent_id: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<String>>> + Send;

    /// Page through a parent's children in editorial order, with a keyset cursor.
    ///
    /// The editorial order index is already keyed `(parent_id, order_label)`, so
    /// this is a native seek rather than a scan-and-slice: cost is proportional
    /// to the page, not to the number of children.
    ///
    /// # Arguments
    ///
    /// * `after_label` - Exclusive cursor. Forward scans resume strictly after
    ///   this label, reverse scans strictly before it. Pass the `order_label` of
    ///   the previous page's last row; `None` starts at the beginning (or the
    ///   end, when `descending`).
    /// * `limit` - Maximum children to return. `None` returns all of them.
    /// * `descending` - Walk the editorial order backwards.
    /// * `max_revision` - MVCC bound for snapshot / time-travel reads.
    ///
    /// # Keyset caveat
    ///
    /// Editorial order is mutable. If a child is reordered from before the
    /// cursor to after it, a later page will show it again; moved the other way,
    /// it may be missed. That is inherent to keyset pagination over a mutable
    /// sort key — callers that need a stable snapshot should pin `max_revision`.
    fn list_ordered_children_page(
        &self,
        scope: StorageScope<'_>,
        parent_id: &str,
        after_label: Option<&str>,
        limit: Option<usize>,
        descending: bool,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<OrderedChild>>> + Send;

    /// Look up one child's current editorial order label.
    ///
    /// Used to materialize the order column on scans that are not themselves
    /// driven by the editorial index. Returns `None` if the child is not present
    /// in the parent's order index.
    fn get_child_order_label(
        &self,
        scope: StorageScope<'_>,
        parent_id: &str,
        child_id: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<String>>> + Send;

    /// Like [`Self::list_ordered_children_page`], but returns the full nodes
    /// paired with their editorial order labels.
    ///
    /// This is what query execution needs: the label materializes the `__order`
    /// column and drives the next page's cursor, and it comes for free because it
    /// is already part of the scanned index key.
    ///
    /// A child whose node cannot be loaded (concurrent delete, or a revision not
    /// yet visible) is skipped, so a page may contain fewer rows than `limit`
    /// without being the last page. Drive pagination from the last returned
    /// label, never from the row count.
    fn list_by_parent_page(
        &self,
        scope: StorageScope<'_>,
        parent_id: &str,
        after_label: Option<&str>,
        limit: Option<usize>,
        descending: bool,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<(models::nodes::Node, String)>>> + Send;

    // ========================================================================
    // Path-based operations
    // ========================================================================

    /// Get a node by its path.
    ///
    /// This is equivalent to get() but uses path as the lookup key.
    fn get_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::Node>>> + Send;

    /// Get a node ID by its path without loading the full node.
    ///
    /// This is optimized for lookups where only the ID is needed (e.g. for graph connections).
    fn get_node_id_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<String>>> + Send;

    /// Delete a node by its path.
    ///
    /// This is equivalent to delete() but uses path as the lookup key.
    fn delete_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        options: DeleteNodeOptions,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    // ========================================================================
    // Utility methods
    // ========================================================================

    /// Count all nodes in a workspace without deserializing node data
    ///
    /// This is a memory-efficient alternative to `list_all().len()` for COUNT(*) queries.
    /// It iterates through keys and counts them without deserializing the full Node objects.
    ///
    /// # Performance
    /// - Memory: O(1) - only stores count, not nodes
    /// - Time: O(n) - must iterate all keys
    /// - For 2M nodes: ~10MB memory vs 1-4GB for list_all()
    ///
    /// # Arguments
    /// * `max_revision` - If Some(rev), count nodes at that revision; if None, count at HEAD
    fn count_all(
        &self,
        scope: StorageScope<'_>,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<usize>> + Send;

    /// Scan all nodes whose path starts with the given prefix (at any depth).
    ///
    /// This is used for efficient `PATH_STARTS_WITH(path, '/house/')` queries.
    /// It returns ALL descendants at any depth, not just direct children.
    ///
    /// # Examples
    /// - `prefix="/house/"` returns `["/house/room", "/house/room/bed", "/house/kitchen"]`
    /// - `prefix="/"` returns ALL nodes in workspace (equivalent to list_all)
    ///
    /// # Performance
    /// Implementation should use RocksDB prefix iterator on PATH_INDEX CF for O(k) performance
    /// where k = number of matching nodes, instead of O(n) where n = all nodes.
    fn scan_by_path_prefix(
        &self,
        scope: StorageScope<'_>,
        path_prefix: &str,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send;

    /// Scan all descendants of a node in tree order (respecting ORDERED_CHILDREN)
    ///
    /// Unlike `scan_by_path_prefix` which returns nodes in lexicographic path order,
    /// this method returns nodes in the order they appear in the tree hierarchy,
    /// respecting the fractional indexing in ORDERED_CHILDREN column family.
    ///
    /// # Arguments
    ///
    /// * `parent_node_id` - The node ID of the parent to scan descendants from
    /// * `options` - List options (includes max_revision for MVCC)
    ///
    /// # Returns
    ///
    /// Nodes in tree traversal order (BFS with ordered children)
    fn scan_descendants_ordered(
        &self,
        scope: StorageScope<'_>,
        parent_node_id: &str,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send;

    /// Like [`Self::scan_descendants_ordered`], but keyset-paginated and
    /// returning each node's `tree_order` — the subtree-wide editorial sort key.
    ///
    /// Traversal is pre-order depth-first (document order). Sorting the returned
    /// `tree_order` values byte-wise reproduces exactly this order, which is what
    /// makes one opaque string serve as both the sort key and the page cursor.
    ///
    /// # Arguments
    ///
    /// * `after_tree_order` - Resume strictly after the node with this
    ///   `tree_order`. Pass the last row's value from the previous page; `None`
    ///   starts at `parent_node_id` itself. Resuming costs O(depth) index seeks,
    ///   not O(nodes already emitted).
    /// * `limit` - Maximum nodes to return. `None` walks the whole subtree.
    ///
    /// # Cursor stability
    ///
    /// Like any keyset cursor over a mutable ordering, a node reordered across
    /// the cursor position may be seen twice or missed. If the cursor's own node
    /// has been deleted or moved since the cursor was issued, the walk resumes
    /// from the still-pending siblings it had already identified rather than
    /// guessing a position. Pin `max_revision` for a stable snapshot.
    fn scan_descendants_ordered_page(
        &self,
        scope: StorageScope<'_>,
        parent_node_id: &str,
        after_tree_order: Option<&str>,
        limit: Option<usize>,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<(models::nodes::Node, String)>>> + Send;

    /// Check if a node has children
    ///
    /// This is more efficient than loading all children just to check if any exist.
    /// Used to populate the `has_children` field in JSON responses.
    ///
    /// # Arguments
    ///
    /// * `node_id` - Node ID to check
    /// * `max_revision` - Optional max revision bound for snapshot isolation
    fn has_children(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    // ========================================================================
    // Tree Operations
    // ========================================================================

    fn move_node(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        new_path: &str,
        operation_meta: Option<models::operations::OperationMeta>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Move a node and ALL its descendants to a new location
    ///
    /// This is like move_node but recursively moves all children as well.
    /// All nodes maintain their IDs but get updated paths.
    fn move_node_tree(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        new_path: &str,
        operation_meta: Option<models::operations::OperationMeta>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn rename_node(
        &self,
        scope: StorageScope<'_>,
        old_path: &str,
        new_name: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    // Deep-children traversal (max_depth is inclusive cap; flatten returns Vec<Node> keyed by path order)
    fn deep_children_nested(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<HashMap<String, models::nodes::DeepNode>>> + Send;

    fn deep_children_flat(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send;

    // DX-friendly array format with nested children
    fn deep_children_array(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::NodeWithChildren>>> + Send;

    // Reordering APIs for a parent's children vector
    fn reorder_child(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        child_name: &str,
        new_position: usize,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn move_child_before(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        child_name: &str,
        before_child_name: &str,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn move_child_after(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        child_name: &str,
        after_child_name: &str,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    // Property access by path
    fn get_property_by_path(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
        property_path: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::properties::PropertyValue>>> + Send;

    fn update_property_by_path(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
        property_path: &str,
        value: models::nodes::properties::PropertyValue,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    // Copy operations (shallow and deep)
    fn copy_node(
        &self,
        scope: StorageScope<'_>,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
        operation_meta: Option<models::operations::OperationMeta>,
    ) -> impl std::future::Future<Output = Result<models::nodes::Node>> + Send;

    fn copy_node_tree(
        &self,
        scope: StorageScope<'_>,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
        operation_meta: Option<models::operations::OperationMeta>,
    ) -> impl std::future::Future<Output = Result<models::nodes::Node>> + Send;

    /// Copy a set of nodes from one branch onto another (branch promotion).
    ///
    /// Unlike [`copy_node_tree`](Self::copy_node_tree) (which mints new ids
    /// within one branch), this copy **preserves node ids**: repeated
    /// promotions update the same target-branch nodes, so the operation is
    /// idempotent per source state. All writes — node blobs, every index
    /// (path / property / reference / relation / ordered-children), carried
    /// translations, and optional pruning — land in ONE atomic WriteBatch
    /// under a single revision, and the target branch HEAD advances to it.
    ///
    /// # Arguments
    /// * `source_branch` / `target_branch` - branches to read from / write to
    /// * `workspace` - workspace the node set lives in
    /// * `roots` - source-branch node paths to copy; each root's **parent
    ///   path must already exist on the target branch** (validation error
    ///   otherwise)
    /// * `recursive` - also copy all descendants of each root
    /// * `delete_missing` - tombstone target-branch nodes under the roots
    ///   whose ids are absent from the copied source set (one-way sync)
    /// * `source_revision` - read the source branch AS OF this revision instead
    ///   of at its head. A promotion of a large set is not instantaneous, so
    ///   without a pin the copy reads whatever each node happens to hold when
    ///   its turn comes and a writer working in parallel lands partly inside
    ///   the result — a torn snapshot, split at whatever boundary the batching
    ///   happened to use. With it the whole promotion sees one consistent
    ///   point in time. The target branch is always resolved at its head: the
    ///   pin says what to copy, never where to put it.
    /// * `operation_meta` - optional actor/message recorded in the revision
    ///   metadata (defaults to a system commit)
    ///
    /// # Errors
    /// - `Error::NotFound` - a branch or a source root doesn't exist
    /// - `Error::Forbidden` - the target branch is protected
    /// - `Error::Validation` - empty roots, identical branches, or a root's
    ///   parent path missing on the target branch
    ///
    /// Backends without branch-crossing storage may return
    /// `Error::Validation` (the in-memory backend does).
    #[allow(clippy::too_many_arguments)]
    fn copy_nodes_across_branches(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        workspace: &str,
        roots: &[String],
        recursive: bool,
        delete_missing: bool,
        source_revision: Option<&raisin_hlc::HLC>,
        operation_meta: Option<models::operations::OperationMeta>,
    ) -> impl std::future::Future<Output = Result<CrossBranchCopySummary>> + Send;

    // Publish/unpublish methods
    fn publish(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn publish_tree(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn unpublish(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    fn unpublish_tree(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    // Fetch published nodes only (where published_at is not null)
    fn get_published(
        &self,
        scope: StorageScope<'_>,
        id: &str,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::Node>>> + Send;

    fn get_published_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::Node>>> + Send;

    fn list_published_children(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send;

    fn list_published_root(
        &self,
        scope: StorageScope<'_>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send;

    // Property-based queries (optional - backends may return empty or fallback to list_all)
    /// Find nodes by exact property value
    ///
    /// **Optional**: Backends may return an empty vec if not supported, in which case
    /// the caller should fallback to list_all() and filter manually.
    fn find_by_property(
        &self,
        _scope: StorageScope<'_>,
        _property_name: &str,
        _property_value: &models::nodes::properties::PropertyValue,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        // Default implementation: return empty (not supported)
        async { Ok(Vec::new()) }
    }

    /// Find nodes that have a specific property (regardless of value)
    ///
    /// **Optional**: Backends may return an empty vec if not supported.
    fn find_nodes_with_property(
        &self,
        _scope: StorageScope<'_>,
        _property_name: &str,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        // Default implementation: return empty (not supported)
        async { Ok(Vec::new()) }
    }

    /// Bulk fetch all descendants of a node using efficient RocksDB prefix scans.
    ///
    /// This method is optimized for building deep trees without recursive individual fetches.
    /// It uses RocksDB prefix iteration on the path index to fetch all descendants in a single scan.
    ///
    /// # Performance
    ///
    /// - O(k) where k = number of descendants (not O(k*log(n)) like individual gets)
    /// - Single RocksDB prefix scan instead of k individual get operations
    /// - Significantly faster for deep trees (10-100x improvement)
    ///
    /// # Arguments
    ///
    /// * `parent_path` - Root path to fetch descendants from (e.g., "/content/articles")
    /// * `max_depth` - Maximum depth to traverse (0 = direct children only, u32::MAX = unlimited)
    /// * `max_revision` - Optional max revision bound for snapshot isolation
    ///
    /// # Returns
    ///
    /// HashMap where key is the full node path and value is the Node.
    /// All descendants up to max_depth are included in a single operation.
    fn get_descendants_bulk(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<HashMap<String, models::nodes::Node>>> + Send;

    /// Validate that parent's NodeType allows child of this type
    ///
    /// This enforces the NodeType.allowed_children schema at the storage layer,
    /// ensuring database consistency regardless of storage implementation.
    ///
    /// # Validation Rules
    /// - If `parent_node_type.allowed_children` is empty -> allow ANY child type
    /// - If `parent_node_type.allowed_children` contains `"*"` -> allow ANY child type
    /// - Otherwise -> `child_node_type` MUST be in the `allowed_children` list
    ///
    /// # Arguments
    /// * `scope` - Branch-level scope (tenant + repo + branch)
    /// * `parent_node_type` - The NodeType name of the parent (e.g., "raisin:Folder")
    /// * `child_node_type` - The NodeType name of the child (e.g., "raisin:Page")
    ///
    /// # Returns
    /// * `Ok(())` if the child type is allowed under this parent type
    /// * `Err(Error::Validation)` if the child type is not allowed
    /// * `Err(Error::NotFound)` if the parent NodeType doesn't exist
    fn validate_parent_allows_child(
        &self,
        scope: BranchScope<'_>,
        parent_node_type: &str,
        child_node_type: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Validate that node type is allowed in workspace
    ///
    /// This enforces the Workspace allowed node types at the storage layer:
    /// - For root nodes (parent = "/"): must be in `allowed_root_node_types`
    /// - For all nodes: must be in `allowed_node_types`
    ///
    /// # Validation Rules
    /// - If node's parent is "/" -> check `workspace.allowed_root_node_types`
    /// - Always check `workspace.allowed_node_types` for all nodes
    /// - Empty lists mean "allow all" (permissive mode)
    /// - "*" wildcard means "allow all"
    fn validate_workspace_allows_node_type(
        &self,
        scope: StorageScope<'_>,
        node_type: &str,
        is_root_node: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// Replay a parent's child ordering from one branch onto another.
///
/// Branch *merge* already carries ordering (it copies the ordered-children
/// index), so the admin-console "create branch → merge back" flow publishes
/// order for free. But a selective, per-node publish (e.g. Studio's SQL-UPSERT
/// publish) never touches the ordering index, so sibling-order changes on the
/// source branch are lost.
///
/// This helper closes that gap: it reads `source_branch`'s visible child order
/// for `parent_path` and replays it onto `target_branch` using the ordinary
/// (self-healing) reorder operations — only for children that exist on both
/// branches. Children present on the target but absent from the source keep
/// their relative position at the end.
///
/// It is generic over any [`NodeRepository`], so transports, host bindings and
/// flow steps can all share one implementation.
#[allow(clippy::too_many_arguments)]
pub async fn apply_child_order_from_branch<R>(
    repo: &R,
    tenant_id: &str,
    repo_id: &str,
    target_branch: &str,
    source_branch: &str,
    workspace: &str,
    parent_path: &str,
    actor: Option<&str>,
) -> Result<()>
where
    R: NodeRepository,
{
    use crate::node_operations::ListOptions;
    use std::collections::HashSet;

    // Source order (in visible / fractional order).
    let source = repo
        .list_children(
            StorageScope::new(tenant_id, repo_id, source_branch, workspace),
            parent_path,
            ListOptions::default(),
        )
        .await?;

    // Names present on the target (only these can be reordered).
    let target = repo
        .list_children(
            StorageScope::new(tenant_id, repo_id, target_branch, workspace),
            parent_path,
            ListOptions::default(),
        )
        .await?;
    let target_names: HashSet<String> = target.into_iter().map(|n| n.name).collect();

    // Walk the source order; sequence the matching target children to match.
    let message = Some("Apply published order");
    let mut prev: Option<String> = None;
    for child in source {
        if !target_names.contains(&child.name) {
            continue;
        }
        let scope = StorageScope::new(tenant_id, repo_id, target_branch, workspace);
        match &prev {
            // First matching child -> move to the front.
            None => {
                repo.reorder_child(scope, parent_path, &child.name, 0, message, actor)
                    .await?;
            }
            // Subsequent children -> place immediately after the previous one.
            Some(p) => {
                repo.move_child_after(scope, parent_path, &child.name, p, message, actor)
                    .await?;
            }
        }
        prev = Some(child.name);
    }

    Ok(())
}
