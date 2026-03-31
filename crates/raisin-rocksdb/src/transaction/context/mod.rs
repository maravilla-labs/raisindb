//! TransactionalContext trait implementation
//!
//! This module organizes the TransactionalContext implementation into focused submodules:
//! - `nodes`: Node CRUD operations (create, read, update, delete, copy, list)
//! - `translations`: Translation operations (read, write)
//! - `workspace`: Workspace configuration
//! - `setters`: Transaction metadata setters
//!
//! # Operations
//!
//! ## Node Operations
//! - `put_node`: Create or update a node (validates and handles both cases)
//! - `add_node`: Optimized path for new nodes (validates as CREATE only)
//! - `delete_node`: Delete a node with tombstone marker
//! - `get_node`: Read a node by ID with read-your-writes semantics
//! - `get_node_by_path`: Read a node by path
//! - `delete_path_index`: Remove a path index entry
//! - `list_children`: List ordered children of a parent node
//! - `copy_node_tree`: Copy an entire node subtree
//!
//! ## Translation Operations
//! - `store_translation`: Store a locale overlay for a node
//! - `get_translation`: Get a locale overlay
//! - `list_translations_for_node`: List all locales for a node
//!
//! ## Workspace Operations
//! - `put_workspace`: Store workspace configuration
//!
//! ## Metadata Operations
//! - `set_branch`: Set the branch for this transaction
//! - `set_actor`: Set the actor (user) performing the transaction
//! - `set_message`: Set the commit message
//! - `set_tenant_repo`: Set tenant and repository IDs
//! - `set_is_manual_version`: Mark as manual version creation
//! - `set_manual_version_node_id`: Set the node ID for manual versioning
//! - `set_is_system`: Mark as system transaction
//!
//! # Read-Your-Writes Semantics
//!
//! All read operations check the in-memory cache first, ensuring that uncommitted
//! changes made earlier in the transaction are visible to later operations.
//!
//! # Lock Scoping
//!
//! All lock acquisitions are carefully scoped using blocks to minimize hold times.
//! Locks are released before async operations to prevent deadlocks.

mod nodes;
mod relation;
mod setters;
mod translations;
mod workspace;

use async_trait::async_trait;
use raisin_error::Result;
use raisin_models::{nodes::Node, translations::LocaleOverlay, workspace::Workspace};
use raisin_storage::transactional::TransactionalContext;

use super::RocksDBTransaction;

#[async_trait]
impl TransactionalContext for RocksDBTransaction {
    /// Create or update a node in the transaction
    ///
    /// This method handles both creates and updates:
    /// - If the node doesn't exist, validates as CREATE
    /// - If the node exists, validates as UPDATE
    ///
    /// # Parent Normalization
    ///
    /// The parent field is normalized from the path before saving. Parent is NEVER null:
    /// - Root-level nodes have parent = "/"
    /// - Other nodes have parent = parent's name
    ///
    /// # Validation
    ///
    /// Uses NodeRepository validation helpers:
    /// - CREATE: Validates parent allows child, workspace allows type
    /// - UPDATE: Validates updates are allowed (with type change support for migrations)
    ///
    /// # Indexes
    ///
    /// Updates all indexes atomically:
    /// - NODES: Node data with versioned key
    /// - PATH_INDEX: Path -> node_id mapping (with tombstone for old path on moves)
    /// - PROPERTY_INDEX: Property value indexes for queries
    /// - REFERENCE_INDEX: Forward and reverse reference indexes
    /// - ORDERED_CHILDREN: Fractional index for ordered children
    ///
    /// # Change Tracking
    ///
    /// Tracks changes for:
    /// - Revision snapshot creation (async background job)
    /// - NodeEvent emission (WebSocket notifications)
    /// - CRDT replication (distributed sync)
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the node
    /// * `node` - The node to create or update
    ///
    /// # Returns
    ///
    /// Ok(()) on success, Error on validation or storage failure
    async fn put_node(&self, workspace: &str, node: &Node) -> Result<()> {
        nodes::put_node(self, workspace, node).await
    }

    /// Create a new node in the transaction (optimized for new nodes)
    ///
    /// This is an optimized version of `put_node` for new nodes only.
    /// It validates as CREATE and skips existence checks.
    ///
    /// # Fast Path
    ///
    /// Unlike `put_node`, this method:
    /// - Only validates as CREATE (no existence check)
    /// - Appends to end of ordered children (no existence check)
    /// - Always tracks as Added operation
    ///
    /// # Read-Your-Writes
    ///
    /// When creating initial_structure children, the parent node may have been
    /// created earlier in this same transaction and only exists in the write batch.
    /// We check the transaction's read cache first for read-your-writes semantics.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the node
    /// * `node` - The node to create
    ///
    /// # Returns
    ///
    /// Ok(()) on success, Error on validation or storage failure
    async fn add_node(&self, workspace: &str, node: &Node) -> Result<()> {
        nodes::add_node(self, workspace, node).await
    }

    /// Upsert a node within the transaction (create or update by PATH)
    ///
    /// This implements true UPSERT semantics:
    /// - If a node exists at the given PATH → UPDATE that node (uses existing ID)
    /// - If no node exists at the PATH → CREATE new node (uses provided ID)
    ///
    /// This differs from `put_node()` which does create-or-update by ID.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the node
    /// * `node` - The node to upsert (path is the key for existence check)
    ///
    /// # Returns
    ///
    /// Ok(()) on success, Error on validation or storage failure
    async fn upsert_node(&self, workspace: &str, node: &Node) -> Result<()> {
        nodes::upsert_node(self, workspace, node).await
    }

    /// Add a node, creating any missing parent folders first.
    ///
    /// This is the deep version of `add_node()`. It ensures all intermediate
    /// folders in the node's path exist before creating the node.
    ///
    /// # Parent Folder Creation
    ///
    /// For path `/lib/raisin/handler/mynode`, this creates:
    /// - `/lib` as `raisin:Folder` (if not exists)
    /// - `/lib/raisin` as `raisin:Folder` (if not exists)
    /// - `/lib/raisin/handler` as `raisin:Folder` (if not exists)
    /// - Then creates the actual node at `/lib/raisin/handler/mynode`
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the node
    /// * `node` - The node to create (must have a valid path)
    /// * `parent_node_type` - Node type for auto-created parent folders
    ///   (e.g., "raisin:Folder")
    ///
    /// # Returns
    ///
    /// Ok(()) on success, Error if node already exists or validation failure
    async fn add_deep_node(
        &self,
        workspace: &str,
        node: &Node,
        parent_node_type: &str,
    ) -> Result<()> {
        nodes::add_deep_node(self, workspace, node, parent_node_type).await
    }

    /// Upsert a node by PATH, creating any missing parent folders first.
    ///
    /// This is the deep version of `upsert_node()`. It ensures all intermediate
    /// folders in the node's path exist before upserting the node.
    ///
    /// # Semantics
    ///
    /// - Missing parent folders are always CREATED (never updated)
    /// - If a node exists at the given PATH → UPDATE that node (preserves existing ID)
    /// - If no node exists at the PATH → CREATE new node (uses provided ID)
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the node
    /// * `node` - The node to create or update
    /// * `parent_node_type` - Node type for auto-created parent folders
    ///   (e.g., "raisin:Folder")
    ///
    /// # Returns
    ///
    /// Ok(()) on success, Error on validation or storage failure
    async fn upsert_deep_node(
        &self,
        workspace: &str,
        node: &Node,
        parent_node_type: &str,
    ) -> Result<()> {
        nodes::upsert_deep_node(self, workspace, node, parent_node_type).await
    }

    /// Delete a node from the transaction
    ///
    /// # WARNING
    ///
    /// This method does NOT check for children or cascade delete.
    /// Caller is responsible for ensuring node has no children, or for calling
    /// delete_descendants() first. This is intentional for transactions because:
    /// 1. Bulk operations (imports, migrations) need fine control over deletion order
    /// 2. Tree deletions should delete children explicitly (makes operation visible in logs)
    /// 3. Transactions can't easily cascade across multiple nodes atomically
    ///
    /// For safe single-node deletes with cascade, use NodeRepository::delete() instead.
    ///
    /// # Tombstoning
    ///
    /// Uses MVCC tombstone marker (b"T") instead of deleting keys.
    /// This preserves time-travel semantics for historical queries.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the node
    /// * `node_id` - The ID of the node to delete
    ///
    /// # Returns
    ///
    /// Ok(()) on success, Error if node not found or storage failure
    async fn delete_node(&self, workspace: &str, node_id: &str) -> Result<()> {
        nodes::delete_node(self, workspace, node_id).await
    }

    /// Get a node by ID with read-your-writes semantics
    ///
    /// Checks the read cache first to ensure uncommitted changes are visible.
    ///
    /// # MVCC Read
    ///
    /// Reads the latest version of the node at or before the branch HEAD.
    /// Skips tombstone markers to respect deletions.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the node
    /// * `node_id` - The ID of the node to read
    ///
    /// # Returns
    ///
    /// Ok(Some(node)) if found, Ok(None) if not found or deleted
    async fn get_node(&self, workspace: &str, node_id: &str) -> Result<Option<Node>> {
        nodes::get_node(self, workspace, node_id).await
    }

    /// Get a node by path with read-your-writes semantics
    ///
    /// Checks the read cache first to ensure uncommitted changes are visible.
    ///
    /// # Path Resolution
    ///
    /// 1. Queries PATH_INDEX to get node_id
    /// 2. Calls get_node to read the node data
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the node
    /// * `path` - The path of the node to read
    ///
    /// # Returns
    ///
    /// Ok(Some(node)) if found, Ok(None) if not found or deleted
    async fn get_node_by_path(&self, workspace: &str, path: &str) -> Result<Option<Node>> {
        nodes::get_node_by_path(self, workspace, path).await
    }

    /// Delete a path index entry with tombstone marker
    ///
    /// # MVCC Semantics
    ///
    /// Writes a tombstone marker (b"T") instead of deleting the key.
    /// This preserves time-travel semantics for historical queries.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the path
    /// * `path` - The path to delete
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    async fn delete_path_index(&self, workspace: &str, path: &str) -> Result<()> {
        nodes::delete_path_index(self, workspace, path).await
    }

    /// Store a translation (locale overlay) for a node
    ///
    /// # Translation Storage
    ///
    /// Translations are stored in two column families:
    /// - TRANSLATION_DATA: The actual LocaleOverlay data
    /// - TRANSLATION_INDEX: Reverse index for listing translations by locale
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the node
    /// * `node_id` - The ID of the node
    /// * `locale` - The locale code (e.g., "en", "fr")
    /// * `overlay` - The locale overlay data
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    async fn store_translation(
        &self,
        workspace: &str,
        node_id: &str,
        locale: &str,
        overlay: LocaleOverlay,
    ) -> Result<()> {
        translations::store_translation(self, workspace, node_id, locale, overlay).await
    }

    /// Get a translation (locale overlay) for a node
    ///
    /// Checks the read cache first for read-your-writes semantics.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the node
    /// * `node_id` - The ID of the node
    /// * `locale` - The locale code (e.g., "en", "fr")
    ///
    /// # Returns
    ///
    /// Ok(Some(overlay)) if found, Ok(None) if not found
    async fn get_translation(
        &self,
        workspace: &str,
        node_id: &str,
        locale: &str,
    ) -> Result<Option<LocaleOverlay>> {
        translations::get_translation(self, workspace, node_id, locale).await
    }

    /// List all available locales for a node
    ///
    /// Returns the set of locale codes that have translations for this node.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the node
    /// * `node_id` - The ID of the node
    ///
    /// # Returns
    ///
    /// Ok(Vec<String>) with locale codes
    async fn list_translations_for_node(
        &self,
        workspace: &str,
        node_id: &str,
    ) -> Result<Vec<String>> {
        translations::list_translations_for_node(self, workspace, node_id).await
    }

    /// List ordered children of a parent node
    ///
    /// Delegates to NodeRepository's list_children which uses the ORDERED_CHILDREN index.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the nodes
    /// * `parent_path` - The path of the parent node
    ///
    /// # Returns
    ///
    /// Ok(Vec<Node>) with children in fractional index order
    async fn list_children(&self, workspace: &str, parent_path: &str) -> Result<Vec<Node>> {
        nodes::list_children(self, workspace, parent_path).await
    }

    /// Reorder a child node to appear before another sibling
    ///
    /// Delegates to NodeRepository's move_child_before which uses fractional indexing.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the nodes
    /// * `parent_path` - The path of the parent node
    /// * `child_name` - The name of the child to move
    /// * `before_child_name` - The name of the sibling to position before
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    async fn reorder_child_before(
        &self,
        workspace: &str,
        parent_path: &str,
        child_name: &str,
        before_child_name: &str,
    ) -> Result<()> {
        nodes::reorder_child_before(self, workspace, parent_path, child_name, before_child_name)
            .await
    }

    /// Reorder a child node to appear after another sibling
    ///
    /// Delegates to NodeRepository's move_child_after which uses fractional indexing.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the nodes
    /// * `parent_path` - The path of the parent node
    /// * `child_name` - The name of the child to move
    /// * `after_child_name` - The name of the sibling to position after
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    async fn reorder_child_after(
        &self,
        workspace: &str,
        parent_path: &str,
        child_name: &str,
        after_child_name: &str,
    ) -> Result<()> {
        nodes::reorder_child_after(self, workspace, parent_path, child_name, after_child_name).await
    }

    /// Copy an entire node tree
    ///
    /// Delegates to NodeRepository's copy_node_tree which handles:
    /// - Recursive copying of all descendants
    /// - ID mapping and reference rewriting
    /// - Fractional index preservation
    /// - Atomic transaction handling
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the nodes
    /// * `source_path` - The path of the source node to copy
    /// * `target_parent` - The path of the target parent
    /// * `new_name` - Optional new name for the copied root node
    /// * `_actor` - The actor performing the operation (unused in transaction)
    ///
    /// # Returns
    ///
    /// Ok(Node) with the copied root node
    async fn copy_node_tree(
        &self,
        workspace: &str,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
        _actor: &str,
    ) -> Result<Node> {
        nodes::copy_node_tree(
            self,
            workspace,
            source_path,
            target_parent,
            new_name,
            _actor,
        )
        .await
    }

    /// Store workspace configuration
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace configuration to store
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    async fn put_workspace(&self, workspace: &Workspace) -> Result<()> {
        workspace::put_workspace(self, workspace).await
    }

    /// Set the branch for this transaction
    ///
    /// All operations will be performed on this branch.
    fn set_branch(&self, branch: &str) -> Result<()> {
        setters::set_branch(self, branch)
    }

    /// Set the actor (user) performing this transaction
    ///
    /// Used for commit metadata and audit logging.
    fn set_actor(&self, actor: &str) -> Result<()> {
        setters::set_actor(self, actor)
    }

    /// Set the commit message for this transaction
    ///
    /// Describes the changes made in this transaction.
    fn set_message(&self, message: &str) -> Result<()> {
        setters::set_message(self, message)
    }

    /// Get the current commit message (if set)
    ///
    /// Returns the message that will be used for this transaction's commit.
    fn get_message(&self) -> Result<Option<String>> {
        setters::get_message(self)
    }

    /// Get the current actor (if set)
    ///
    /// Returns the actor (user) performing this transaction.
    fn get_actor(&self) -> Result<Option<String>> {
        setters::get_actor(self)
    }

    /// Set tenant and repository IDs for this transaction
    ///
    /// All operations will be scoped to this tenant and repository.
    fn set_tenant_repo(&self, tenant_id: &str, repo_id: &str) -> Result<()> {
        setters::set_tenant_repo(self, tenant_id, repo_id)
    }

    /// Mark this transaction as manual version creation
    ///
    /// Used for explicit versioning operations.
    fn set_is_manual_version(&self, is_manual: bool) -> Result<()> {
        setters::set_is_manual_version(self, is_manual)
    }

    /// Set the node ID for manual versioning
    ///
    /// Identifies the node being manually versioned.
    fn set_manual_version_node_id(&self, node_id: &str) -> Result<()> {
        setters::set_manual_version_node_id(self, node_id)
    }

    /// Mark this transaction as a system transaction
    ///
    /// System transactions are created by background jobs, migrations, etc.
    /// and may have different validation or auditing rules.
    fn set_is_system(&self, is_system: bool) -> Result<()> {
        setters::set_is_system(self, is_system)
    }

    fn set_auth_context(&self, auth_context: raisin_models::auth::AuthContext) -> Result<()> {
        setters::set_auth_context(self, auth_context)
    }

    fn get_auth_context(&self) -> Result<Option<std::sync::Arc<raisin_models::auth::AuthContext>>> {
        setters::get_auth_context(self)
    }

    /// Set schema validation toggle
    ///
    /// When enabled (default), node operations are validated against their
    /// NodeType, Archetype, and ElementType schemas.
    ///
    /// Disable this for bulk imports or migrations where validation should be skipped.
    fn set_validate_schema(&self, enabled: bool) -> Result<()> {
        self.set_validate_schema_enabled(enabled);
        Ok(())
    }

    /// Check if schema validation is enabled
    ///
    /// Returns true (default) if schema validation is enabled.
    fn validate_schema(&self) -> bool {
        self.is_validate_schema_enabled()
    }

    /// Add a relationship from source node to target node within the transaction
    ///
    /// Creates both forward (outgoing) and reverse (incoming) index entries.
    /// The relationship is versioned at the current HEAD revision and will be
    /// tracked for replication via the ChangeTracker.
    ///
    /// # Arguments
    ///
    /// * `source_workspace` - Workspace containing the source node
    /// * `source_node_id` - ID of the source node
    /// * `source_node_type` - Node type of the source (e.g., "raisin:Page")
    /// * `relation` - RelationRef containing target details and relation_id
    async fn add_relation(
        &self,
        source_workspace: &str,
        source_node_id: &str,
        source_node_type: &str,
        relation: raisin_models::nodes::RelationRef,
    ) -> Result<()> {
        relation::add_relation(
            self,
            source_workspace,
            source_node_id,
            source_node_type,
            relation,
        )
        .await
    }

    /// Remove a specific relationship between two nodes within the transaction
    ///
    /// Removes both forward and reverse index entries for this relationship.
    /// The removal is tracked for replication via the ChangeTracker.
    async fn remove_relation(
        &self,
        source_workspace: &str,
        source_node_id: &str,
        target_workspace: &str,
        target_node_id: &str,
    ) -> Result<bool> {
        relation::remove_relation(
            self,
            source_workspace,
            source_node_id,
            target_workspace,
            target_node_id,
        )
        .await
    }

    /// Scan all nodes in a workspace (collects all into memory)
    ///
    /// This is used for management operations like re-indexing and integrity checks
    /// that need to iterate over all nodes in a workspace.
    ///
    /// For bulk UPDATE/DELETE operations with complex WHERE clauses, use the SQL
    /// execution engine which leverages optimized SELECT queries to find matching
    /// nodes efficiently (via property indexes, full-text search, etc.) before updating.
    ///
    /// # Warning
    ///
    /// This loads ALL nodes into memory at once. For large datasets (100K+ nodes),
    /// this can cause high memory usage.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace to scan
    ///
    /// # Returns
    ///
    /// Ok(Vec<Node>) with all nodes in the workspace
    async fn scan_nodes(&self, workspace: &str) -> Result<Vec<Node>> {
        nodes::scan_nodes(self, workspace).await
    }

    /// Move a node and all its descendants to a new location within the transaction
    ///
    /// This method moves a node tree (node + all descendants) to a new parent path.
    /// All nodes maintain their IDs but get updated paths.
    ///
    /// # Arguments
    ///
    /// * `workspace` - The workspace containing the nodes
    /// * `node_id` - The ID of the node to move (root of the tree)
    /// * `new_path` - The new path for the node (e.g., "/new-parent/node-name")
    ///
    /// # Returns
    ///
    /// Ok(()) on success
    async fn move_node_tree(&self, workspace: &str, node_id: &str, new_path: &str) -> Result<()> {
        nodes::move_node_tree(self, workspace, node_id, new_path).await
    }

    /// Commit the transaction
    ///
    /// Delegates to the Transaction trait implementation.
    async fn commit(&self) -> Result<()> {
        raisin_storage::Transaction::commit(self).await
    }

    /// Rollback the transaction
    ///
    /// Delegates to the Transaction trait implementation.
    async fn rollback(&self) -> Result<()> {
        raisin_storage::Transaction::rollback(self).await
    }
}
