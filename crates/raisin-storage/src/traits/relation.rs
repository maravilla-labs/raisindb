// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Relation repository trait definitions.
//!
//! This module contains the `RelationRepository` trait for tracking
//! graph relationships between nodes.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models as models;

use crate::scope::{BranchScope, StorageScope};

/// Global relation scan result: (source_workspace, source_id, target_workspace, target_id, relation)
pub type GlobalRelationEntry = (String, String, String, String, models::nodes::FullRelation);

/// Relationship indexing repository for tracking graph relationships between nodes.
///
/// This trait provides a consistent interface for managing bidirectional relationships
/// across all storage backends, enabling:
/// - Outgoing relationships: Which nodes does this node relate to?
/// - Incoming relationships: Which nodes relate to this node?
/// - Cross-workspace relationships: Nodes can relate to nodes in other workspaces
/// - Revision-aware queries: Relationships support time-travel via max_revision
///
/// # Scoped Architecture
///
/// Methods that operate within a single workspace use `StorageScope` (tenant + repo + branch + workspace).
/// Methods that span workspaces or only need branch-level context use `BranchScope` (tenant + repo + branch).
///
/// # Implementation Notes
///
/// - **Tenant Isolation**: All methods must respect tenant context
/// - **No Publish State**: Relationships don't have draft/published separation (Phase 1)
/// - **Bidirectional Indexes**: Maintain both forward (out) and reverse (in) indexes
/// - **Revision-Aware**: Support MVCC time-travel queries
/// - **Cross-Workspace**: Source and target can be in different workspaces
///
/// # Key Formats (Backend-Specific)
///
/// RocksDB example:
/// - Forward: `{tenant}\0{repo}\0{branch}\0{workspace}\0rel_out\0{source_id}\0{target_ws}\0{target_id}\0{~rev}`
/// - Reverse: `{tenant}\0{repo}\0{branch}\0{target_ws}\0rel_in\0{target_id}\0{source_ws}\0{source_id}\0{~rev}`
pub trait RelationRepository: Send + Sync {
    /// Add a relationship from source node to target node
    ///
    /// Creates both forward (outgoing) and reverse (incoming) index entries.
    /// The relationship is versioned at the current HEAD revision.
    ///
    /// # Arguments
    ///
    /// * `scope` - Workspace scope for the source node (tenant + repo + branch + source_workspace)
    /// * `source_node_id` - ID of the source node (the one creating the relationship)
    /// * `source_node_type` - Node type of the source (e.g., "raisin:Page")
    /// * `relation` - RelationRef containing target details (target ID, workspace, node type, semantic type, weight)
    ///
    /// # Notes
    ///
    /// - Idempotent: Adding the same relationship multiple times is safe
    /// - Atomic: All three indexes (forward, reverse, global) are updated together
    fn add_relation(
        &self,
        scope: StorageScope<'_>,
        source_node_id: &str,
        source_node_type: &str,
        relation: models::nodes::RelationRef,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Remove a specific relationship between two nodes
    ///
    /// Removes both forward and reverse index entries for this relationship.
    ///
    /// # Arguments
    ///
    /// * `scope` - Workspace scope for the source node (tenant + repo + branch + source_workspace)
    /// * `source_node_id` - ID of the source node
    /// * `target_workspace` - Workspace containing the target node
    /// * `target_node_id` - ID of the target node
    ///
    /// # Returns
    ///
    /// `true` if the relationship existed and was removed, `false` if it didn't exist
    fn remove_relation(
        &self,
        scope: StorageScope<'_>,
        source_node_id: &str,
        target_workspace: &str,
        target_node_id: &str,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Get all outgoing relationships from a node
    ///
    /// Returns relationships visible at the specified revision (or HEAD if None).
    ///
    /// # Arguments
    ///
    /// * `scope` - Workspace scope (tenant + repo + branch + workspace)
    /// * `node_id` - ID of the source node
    /// * `max_revision` - Optional maximum revision (None = HEAD)
    ///
    /// # Returns
    ///
    /// Vector of RelationRef entries describing each outgoing relationship
    fn get_outgoing_relations(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::RelationRef>>> + Send;

    /// Get all incoming relationships to a node
    ///
    /// Returns relationships from other nodes that point to this node,
    /// visible at the specified revision (or HEAD if None).
    ///
    /// # Arguments
    ///
    /// * `scope` - Workspace scope (tenant + repo + branch + workspace)
    /// * `node_id` - ID of the target node
    /// * `max_revision` - Optional maximum revision (None = HEAD)
    ///
    /// # Returns
    ///
    /// Vector of tuples: (source_workspace, source_node_id, relation_ref)
    fn get_incoming_relations(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<(String, String, models::nodes::RelationRef)>>>
           + Send;

    /// Get outgoing relationships filtered by target node type
    ///
    /// Returns only relationships where the target node matches the specified type.
    ///
    /// # Arguments
    ///
    /// * `scope` - Workspace scope (tenant + repo + branch + workspace)
    /// * `node_id` - ID of the source node
    /// * `target_node_type` - Filter by this node type (e.g., "raisin:Page", "raisin:Asset")
    /// * `max_revision` - Optional maximum revision (None = HEAD)
    ///
    /// # Returns
    ///
    /// Vector of RelationRef entries where relation_type matches target_node_type
    fn get_relations_by_type(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        target_node_type: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::RelationRef>>> + Send;

    /// Remove all relationships for a node (both incoming and outgoing)
    ///
    /// This is called during node deletion to maintain referential integrity.
    /// Removes:
    /// - All outgoing relationships FROM this node
    /// - All incoming relationships TO this node
    ///
    /// # Arguments
    ///
    /// * `scope` - Workspace scope (tenant + repo + branch + workspace)
    /// * `node_id` - ID of the node being deleted
    ///
    /// # Notes
    ///
    /// - This operation should be called before deleting the node itself
    /// - Creates tombstone entries at the current revision to maintain history
    fn remove_all_relations_for_node(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Scan all relationships across all workspaces, optionally filtered by type
    ///
    /// This method uses the global relationship index to efficiently query relationships
    /// across workspace boundaries without knowing source nodes upfront. It's optimized
    /// for Cypher graph queries that scan by relation type.
    ///
    /// # Use Cases
    ///
    /// - Cypher queries: `MATCH (a)-[:TYPE]->(b)` across all workspaces
    /// - Cross-workspace graph traversal without specific starting nodes
    /// - Global relationship analytics and statistics
    ///
    /// # Arguments
    ///
    /// * `scope` - Branch scope (tenant + repo + branch)
    /// * `relation_type_filter` - Optional filter by relation type (e.g., "ntRaisinFolder")
    ///   If None, returns all relationships regardless of type
    /// * `max_revision` - Optional maximum revision (None = HEAD)
    ///
    /// # Returns
    ///
    /// Vector of tuples: `(source_workspace, source_id, target_workspace, target_id, relation_ref)`
    ///
    /// # Performance
    ///
    /// - With type filter: O(relationships_of_type) - uses prefix scan on global index
    /// - Without type filter: O(all_relationships) - scans entire global index
    fn scan_relations_global(
        &self,
        scope: BranchScope<'_>,
        relation_type_filter: Option<&str>,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<GlobalRelationEntry>>> + Send;
}
