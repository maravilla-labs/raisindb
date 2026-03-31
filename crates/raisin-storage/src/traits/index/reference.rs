// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Reference index repository trait for tracking PropertyValue::Reference relationships

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models as models;
use std::collections::HashMap;

use crate::scope::StorageScope;

/// Reference indexing repository for tracking PropertyValue::Reference relationships.
///
/// This trait provides a consistent interface for indexing reference properties
/// across all storage backends, enabling fast lookups of:
/// - Which nodes reference a specific target node
/// - What references a node contains
/// - Efficient resolution of duplicate references
///
/// # Scoped Architecture
///
/// All methods take a `StorageScope` (tenant + repo + branch + workspace).
///
/// # Implementation Notes
///
/// - **Tenant Isolation**: All methods must respect tenant context when present
/// - **Publish Separation**: Draft and published references use separate index spaces
/// - **Synchronous Updates**: Index updates happen inline during storage operations
/// - **Path Tracking**: Store exact property paths (e.g., "hero.image", "items.0.asset")
/// - **Bidirectional Indexes**: Maintain both forward (source->target) and reverse (target->source) indexes
///
/// # Key Formats (Backend-Specific)
///
/// RocksDB example:
/// - Forward: `/{tenant_id}/{deployment}ref:{workspace}:{node_id}:{property_path}` -> RaisinReference
/// - Reverse: `/{tenant_id}/{deployment}ref_rev:{target_workspace}:{target_path}:{source_node_id}:{property_path}` -> ""
/// - Published versions use `ref_pub` and `ref_rev_pub` prefixes
pub trait ReferenceIndexRepository: Send + Sync {
    /// Index all references in a node's properties
    ///
    /// Extracts all PropertyValue::Reference instances from properties and stores:
    /// - Forward index: source_node -> [(property_path, target_reference)]
    /// - Reverse index: target_reference -> [(source_node_id, property_path)]
    fn index_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, models::nodes::properties::PropertyValue>,
        revision: &HLC,
        is_published: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Remove all reference indexes for a node (writes tombstones for MVCC)
    ///
    /// Called before node delete to write tombstones for all reference indexes.
    fn unindex_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, models::nodes::properties::PropertyValue>,
        revision: &HLC,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Update publish status for a node's reference indexes
    ///
    /// Called on publish/unpublish to write new indexes at new revision.
    fn update_reference_publish_status(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, models::nodes::properties::PropertyValue>,
        revision: &HLC,
        is_published: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Find all nodes that reference a specific target
    ///
    /// Uses reverse index for O(1) or O(log n) lookup.
    ///
    /// # Returns
    /// Vector of (source_node_id, property_path) tuples
    fn find_referencing_nodes(
        &self,
        scope: StorageScope<'_>,
        target_workspace: &str,
        target_path: &str,
        published_only: bool,
    ) -> impl std::future::Future<Output = Result<Vec<(String, String)>>> + Send;

    /// Get all references from a specific node
    ///
    /// Uses forward index for O(1) or O(log n) lookup.
    ///
    /// # Returns
    /// Vector of (property_path, RaisinReference) tuples
    fn get_node_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        published_only: bool,
    ) -> impl std::future::Future<
        Output = Result<Vec<(String, models::nodes::properties::RaisinReference)>>,
    > + Send;

    /// Get unique target references from a node (deduplicated)
    ///
    /// Groups multiple references to the same target together for efficient resolution.
    ///
    /// # Returns
    /// HashMap<target_key, (property_paths, RaisinReference)> where:
    /// - target_key: "{workspace}:{path}" uniquely identifies the target
    /// - property_paths: All paths in the node that reference this target
    /// - RaisinReference: The reference details
    fn get_unique_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        published_only: bool,
    ) -> impl std::future::Future<
        Output = Result<HashMap<String, (Vec<String>, models::nodes::properties::RaisinReference)>>,
    > + Send;
}
