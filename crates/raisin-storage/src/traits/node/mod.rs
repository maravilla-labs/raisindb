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
use std::collections::HashMap;

use crate::node_operations::{
    CreateNodeOptions, DeleteNodeOptions, ListOptions, NodeWithPopulatedChildren, UpdateNodeOptions,
};
use crate::scope::{BranchScope, StorageScope};

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
