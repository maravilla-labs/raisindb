// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Transactional operation support for storage
//!
//! This module provides traits and utilities for performing transactional
//! operations across storage backends.

use async_trait::async_trait;
use raisin_error::Result;
use raisin_models::{nodes::Node, translations::LocaleOverlay, workspace::Workspace};

/// Context for transactional operations
///
/// This wraps a transaction and provides high-level methods for
/// performing storage operations within the transaction context.
#[async_trait]
pub trait TransactionalContext: Send + Sync {
    /// Put a node within the transaction
    async fn put_node(&self, workspace: &str, node: &Node) -> Result<()>;

    /// Add a brand new node within the transaction (optimized)
    ///
    /// This is an optimized version of `put_node()` for new nodes.
    /// It skips existence checks at the storage layer for better performance.
    async fn add_node(&self, workspace: &str, node: &Node) -> Result<()>;

    /// Upsert a node within the transaction (create or update by PATH)
    ///
    /// This implements true UPSERT semantics:
    /// - If a node exists at the given PATH → UPDATE that node (uses existing ID)
    /// - If no node exists at the PATH → CREATE new node (uses provided ID)
    ///
    /// This differs from `put_node()` which does create-or-update by ID.
    async fn upsert_node(&self, workspace: &str, node: &Node) -> Result<()>;

    /// Add a node, creating any missing parent folders first.
    ///
    /// This is the deep version of `add_node()`. It ensures all intermediate
    /// folders in the node's path exist before creating the node.
    ///
    /// # Arguments
    /// * `workspace` - The workspace identifier
    /// * `node` - The node to create (must have a valid path)
    /// * `parent_node_type` - The node type to use for auto-created parent folders
    ///   (e.g., "raisin:Folder")
    ///
    /// # Errors
    /// Returns an error if the node already exists at the path.
    /// Use `upsert_deep_node` for create-or-update semantics.
    async fn add_deep_node(
        &self,
        workspace: &str,
        node: &Node,
        parent_node_type: &str,
    ) -> Result<()>;

    /// Upsert a node by PATH, creating any missing parent folders first.
    ///
    /// This is the deep version of `upsert_node()`. It ensures all intermediate
    /// folders in the node's path exist before upserting the node.
    ///
    /// # Arguments
    /// * `workspace` - The workspace identifier
    /// * `node` - The node to create or update
    /// * `parent_node_type` - The node type to use for auto-created parent folders
    ///   (e.g., "raisin:Folder")
    ///
    /// # Semantics
    /// - If a node exists at the given PATH → UPDATE that node (preserves existing ID)
    /// - If no node exists at the PATH → CREATE new node (uses provided ID)
    /// - Missing parent folders are always created (never updated)
    async fn upsert_deep_node(
        &self,
        workspace: &str,
        node: &Node,
        parent_node_type: &str,
    ) -> Result<()>;

    /// Delete a node within the transaction
    async fn delete_node(&self, workspace: &str, node_id: &str) -> Result<()>;

    /// Get a node within the transaction (sees uncommitted changes)
    async fn get_node(&self, workspace: &str, node_id: &str) -> Result<Option<Node>>;

    /// Get a node by path within the transaction (sees uncommitted changes)
    async fn get_node_by_path(&self, workspace: &str, path: &str) -> Result<Option<Node>>;

    /// Delete a path index entry within the transaction
    ///
    /// This is useful for rename/move operations where the old path index needs to be removed
    /// before the node is updated with a new path.
    async fn delete_path_index(&self, workspace: &str, path: &str) -> Result<()>;

    /// Store a translation within the transaction
    ///
    /// Creates or updates a translation overlay for a node in a specific locale.
    /// The translation is part of the transaction and will be committed/rolled back
    /// together with other operations.
    ///
    /// # Arguments
    /// * `workspace` - The workspace identifier
    /// * `node_id` - The node to translate
    /// * `locale` - The locale code (e.g., "fr", "en-US")
    /// * `overlay` - The translation overlay (Properties or Hidden)
    async fn store_translation(
        &self,
        workspace: &str,
        node_id: &str,
        locale: &str,
        overlay: LocaleOverlay,
    ) -> Result<()>;

    /// Get a translation within the transaction (sees uncommitted changes)
    ///
    /// Retrieves a translation overlay for a node in a specific locale.
    /// This method supports read-your-writes semantics, meaning it will
    /// see translations stored earlier in the same transaction.
    ///
    /// # Arguments
    /// * `workspace` - The workspace identifier
    /// * `node_id` - The node to get translation for
    /// * `locale` - The locale code
    ///
    /// # Returns
    /// * `Ok(Some(overlay))` - Translation exists
    /// * `Ok(None)` - No translation for this locale
    async fn get_translation(
        &self,
        workspace: &str,
        node_id: &str,
        locale: &str,
    ) -> Result<Option<LocaleOverlay>>;

    /// List all translation locales for a node within the transaction
    ///
    /// Returns all locale codes that have translations for the specified node.
    /// Includes both committed translations and uncommitted changes in this transaction.
    ///
    /// # Arguments
    /// * `workspace` - The workspace identifier
    /// * `node_id` - The node to list translations for
    ///
    /// # Returns
    /// Vector of locale codes (e.g., ["fr", "de", "en-US"])
    async fn list_translations_for_node(
        &self,
        workspace: &str,
        node_id: &str,
    ) -> Result<Vec<String>>;

    /// List children of a node by parent path
    ///
    /// Returns all child nodes in fractional index order.
    /// This method queries the ORDERED_CHILDREN index and returns full Node objects.
    ///
    /// # Arguments
    /// * `workspace` - The workspace identifier
    /// * `parent_path` - The parent node's path (e.g., "/blog" or "/" for root)
    ///
    /// # Returns
    /// Vector of child Node objects in fractional index order
    async fn list_children(&self, workspace: &str, parent_path: &str) -> Result<Vec<Node>>;

    /// Reorder a child node to appear before another sibling within the transaction
    ///
    /// Moves the child node to appear immediately before the target sibling
    /// in the parent's child ordering. Uses fractional indexing for efficient O(1) reordering.
    ///
    /// # Arguments
    /// * `workspace` - The workspace identifier
    /// * `parent_path` - The parent node's path (e.g., "/blog" or "/" for root)
    /// * `child_name` - The name of the child node to move
    /// * `before_child_name` - The name of the sibling to position before
    ///
    /// # Errors
    /// Returns an error if either child doesn't exist under the specified parent
    async fn reorder_child_before(
        &self,
        workspace: &str,
        parent_path: &str,
        child_name: &str,
        before_child_name: &str,
    ) -> Result<()>;

    /// Reorder a child node to appear after another sibling within the transaction
    ///
    /// Moves the child node to appear immediately after the target sibling
    /// in the parent's child ordering. Uses fractional indexing for efficient O(1) reordering.
    ///
    /// # Arguments
    /// * `workspace` - The workspace identifier
    /// * `parent_path` - The parent node's path (e.g., "/blog" or "/" for root)
    /// * `child_name` - The name of the child node to move
    /// * `after_child_name` - The name of the sibling to position after
    ///
    /// # Errors
    /// Returns an error if either child doesn't exist under the specified parent
    async fn reorder_child_after(
        &self,
        workspace: &str,
        parent_path: &str,
        child_name: &str,
        after_child_name: &str,
    ) -> Result<()>;

    /// Copy a node tree (recursive) within the transaction
    ///
    /// This method delegates to the storage layer's optimized copy_node_tree implementation
    /// which handles all children recursively, translations, and fractional index ordering.
    ///
    /// # Arguments
    /// * `workspace` - The workspace identifier
    /// * `source_path` - Path of the node to copy (including all descendants)
    /// * `target_parent` - Path where the copy should be placed
    /// * `new_name` - Optional new name for the copied root node
    /// * `actor` - User performing the operation
    ///
    /// # Returns
    /// The copied root node
    async fn copy_node_tree(
        &self,
        workspace: &str,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
        actor: &str,
    ) -> Result<Node>;

    /// Put a workspace within the transaction
    async fn put_workspace(&self, workspace: &Workspace) -> Result<()>;

    /// Set the branch for this transaction (for revision tracking)
    fn set_branch(&self, branch: &str) -> Result<()>;

    /// Set the actor (user ID) for this transaction
    fn set_actor(&self, actor: &str) -> Result<()>;

    /// Set the commit message for this transaction
    fn set_message(&self, message: &str) -> Result<()>;

    /// Get the current commit message (if set)
    fn get_message(&self) -> Result<Option<String>>;

    /// Get the current actor (if set)
    fn get_actor(&self) -> Result<Option<String>>;

    /// Set tenant and repository IDs for this transaction
    fn set_tenant_repo(&self, tenant_id: &str, repo_id: &str) -> Result<()>;

    /// Set whether this commit is a manual version
    fn set_is_manual_version(&self, is_manual: bool) -> Result<()>;

    /// Set the node ID this manual version applies to
    fn set_manual_version_node_id(&self, node_id: &str) -> Result<()>;

    /// Set whether this is a system commit (background job, migration, etc.)
    fn set_is_system(&self, is_system: bool) -> Result<()>;

    /// Set the authentication context for this transaction.
    ///
    /// When set, RLS (row-level security) and field-level security will be
    /// enforced for all operations in this transaction.
    ///
    /// # Arguments
    ///
    /// * `auth_context` - The authentication context containing user identity and permissions
    fn set_auth_context(&self, auth_context: raisin_models::auth::AuthContext) -> Result<()>;

    /// Get the current authentication context (if set).
    ///
    /// Returns None if no auth context has been set (system/anonymous context).
    fn get_auth_context(&self) -> Result<Option<std::sync::Arc<raisin_models::auth::AuthContext>>>;

    /// Set schema validation toggle.
    ///
    /// When enabled (default), node operations are validated against their
    /// NodeType, Archetype, and ElementType schemas. This includes:
    /// - Required fields in NodeTypes
    /// - Required fields in Archetypes
    /// - Required fields in ElementTypes (for nested elements)
    /// - Strict mode validation (no undefined properties)
    ///
    /// Disable this for bulk imports or migrations where validation should be skipped.
    ///
    /// # Arguments
    ///
    /// * `enabled` - Whether to enable schema validation (default: true)
    fn set_validate_schema(&self, enabled: bool) -> Result<()>;

    /// Check if schema validation is enabled.
    ///
    /// Returns true (default) if schema validation is enabled,
    /// false if it has been disabled via `set_validate_schema(false)`.
    fn validate_schema(&self) -> bool;

    /// Add a relationship from source node to target node within the transaction
    ///
    /// Creates both forward (outgoing) and reverse (incoming) index entries.
    /// The relationship is versioned at the current HEAD revision and will be
    /// tracked for replication.
    ///
    /// # Arguments
    ///
    /// * `source_workspace` - Workspace containing the source node
    /// * `source_node_id` - ID of the source node (the one creating the relationship)
    /// * `source_node_type` - Node type of the source (e.g., "raisin:Page")
    /// * `relation` - RelationRef containing target details (target ID, workspace, node type, semantic type, weight, relation_id)
    ///
    /// # Notes
    ///
    /// - The relation.relation_id will be preserved for CRDT Add-Wins semantics
    /// - Changes are tracked in the transaction's ChangeTracker for replication
    /// - All three indexes (forward, reverse, global) are updated atomically
    async fn add_relation(
        &self,
        source_workspace: &str,
        source_node_id: &str,
        source_node_type: &str,
        relation: raisin_models::nodes::RelationRef,
    ) -> Result<()>;

    /// Remove a specific relationship between two nodes within the transaction
    ///
    /// Removes both forward and reverse index entries for this relationship.
    /// The removal is tracked for replication.
    ///
    /// # Arguments
    ///
    /// * `source_workspace` - Workspace containing the source node
    /// * `source_node_id` - ID of the source node
    /// * `target_workspace` - Workspace containing the target node
    /// * `target_node_id` - ID of the target node
    ///
    /// # Returns
    ///
    /// `true` if the relationship existed and was removed, `false` if it didn't exist
    async fn remove_relation(
        &self,
        source_workspace: &str,
        source_node_id: &str,
        target_workspace: &str,
        target_node_id: &str,
    ) -> Result<bool>;

    /// Scan all nodes in a workspace (collects all into memory)
    ///
    /// This method is used for management operations like re-indexing and integrity checks
    /// that need to iterate over all nodes in a workspace.
    ///
    /// For bulk UPDATE/DELETE operations with complex WHERE clauses, use the SQL
    /// execution engine which leverages optimized SELECT queries to find matching
    /// nodes efficiently (via property indexes, full-text search, etc.) before updating.
    ///
    /// # Arguments
    /// * `workspace` - The workspace identifier
    ///
    /// # Returns
    /// A vector of all nodes in the workspace
    ///
    /// # Performance
    /// **WARNING**: This loads ALL nodes into memory at once. For large datasets
    /// (100K+ nodes), this can cause high memory usage.
    async fn scan_nodes(&self, workspace: &str) -> Result<Vec<Node>>;

    /// Move a node and all its descendants to a new location within the transaction
    ///
    /// This method moves a node tree (node + all descendants) to a new parent path.
    /// All nodes maintain their IDs but get updated paths. The move operation is
    /// performed within the transaction context and will be committed/rolled back
    /// together with other operations.
    ///
    /// # Arguments
    /// * `workspace` - The workspace identifier
    /// * `node_id` - The ID of the node to move (root of the tree)
    /// * `new_path` - The new path for the node (e.g., "/new-parent/node-name")
    ///
    /// # Notes
    /// - This is an O(1) operation for the move itself (path update)
    /// - Index updates for descendants are handled by the storage layer
    /// - The source node must exist
    /// - The target parent path must exist
    async fn move_node_tree(&self, workspace: &str, node_id: &str, new_path: &str) -> Result<()>;

    /// Commit all changes in the transaction
    async fn commit(&self) -> Result<()>;

    /// Rollback all changes in the transaction
    async fn rollback(&self) -> Result<()>;
}

/// Extension trait for Storage to support transactional operations
#[async_trait]
pub trait TransactionalStorage: crate::Storage {
    /// Begin a new transactional context
    async fn begin_context(&self) -> Result<Box<dyn TransactionalContext>>;

    /// Execute a function within a transaction, automatically committing on success
    /// or rolling back on error
    async fn with_transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(
                Box<dyn TransactionalContext>,
            )
                -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<R>> + Send>>
            + Send,
        R: Send + 'static,
    {
        let ctx = self.begin_context().await?;
        f(ctx).await
    }
}

/// Macro to execute multiple operations in a transaction
///
/// # Example
/// ```rust,ignore
/// transact!(storage, ctx => {
///     ctx.put_node("workspace", &node1).await?;
///     ctx.put_node("workspace", &node2).await?;
///     ctx.update_root_children("workspace", vec!["node1".to_string()]).await?;
/// });
/// ```
#[macro_export]
macro_rules! transact {
    ($storage:expr, $ctx:ident => $body:block) => {{
        use $crate::transactional::TransactionalStorage;
        let $ctx = $storage.begin_context().await?;
        let result = (|| async { $body })().await;
        match result {
            Ok(val) => {
                $ctx.commit().await?;
                Ok(val)
            }
            Err(e) => {
                $ctx.rollback().await?;
                Err(e)
            }
        }
    }};
}
