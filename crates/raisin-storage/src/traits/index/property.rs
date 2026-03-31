// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Property index repository trait for fast property-based lookups

use raisin_error::Result;
use raisin_models as models;
use std::collections::HashMap;

use crate::scope::StorageScope;

/// Result entry returned by ordered property scans
#[derive(Debug, Clone)]
pub struct PropertyScanEntry {
    /// Node identifier
    pub node_id: String,
    /// Raw property value (as stored in the index)
    pub property_value: String,
}

/// Property indexing repository for fast property-based lookups.
///
/// This trait provides a consistent interface for indexing node properties
/// across all storage backends (RocksDB, InMemory, PostgreSQL, MongoDB).
///
/// # Scoped Architecture
///
/// All methods take a `StorageScope` (tenant + repo + branch + workspace).
///
/// # Implementation Notes
///
/// - **Tenant Isolation**: All methods must respect tenant context when present
/// - **Publish Separation**: Draft and published content use separate index spaces
/// - **Synchronous Updates**: Index updates happen inline during storage operations
/// - **Performance**: Implementations should provide O(1) or O(log n) lookups
///
/// # Key Formats (Backend-Specific)
///
/// RocksDB example:
/// - Draft: `/{tenant_id}/{deployment}prop:{workspace}:{property_name}:{value_hash}:{node_id}`
/// - Published: `/{tenant_id}/{deployment}prop_pub:{workspace}:{property_name}:{value_hash}:{node_id}`
pub trait PropertyIndexRepository: Send + Sync {
    /// Index properties for a node
    ///
    /// Called after node create/update to add properties to the index.
    fn index_properties(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, models::nodes::properties::PropertyValue>,
        is_published: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Remove all property indexes for a node
    ///
    /// Called before node delete to remove from both draft and published indexes.
    fn unindex_properties(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Update publish status for a node's indexes
    ///
    /// Called on publish/unpublish to move indexes between draft and published spaces.
    fn update_publish_status(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, models::nodes::properties::PropertyValue>,
        is_published: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Find node IDs by exact property value
    ///
    /// Provides O(1) or O(log n) lookup for nodes with specific property values.
    ///
    /// # Returns
    /// Vector of node IDs matching the property value
    fn find_by_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        property_value: &models::nodes::properties::PropertyValue,
        published_only: bool,
    ) -> impl std::future::Future<Output = Result<Vec<String>>> + Send;

    /// Find node IDs that have a specific property (any value)
    ///
    /// Useful for existence queries.
    ///
    /// # Returns
    /// Vector of node IDs that have the property
    fn find_nodes_with_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        published_only: bool,
    ) -> impl std::future::Future<Output = Result<Vec<String>>> + Send;

    /// Find node IDs by property value with optional limit
    ///
    /// Same as find_by_property but supports early termination via limit parameter.
    /// This enables LIMIT pushdown for property index scans.
    fn find_by_property_with_limit(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        property_value: &models::nodes::properties::PropertyValue,
        published_only: bool,
        limit: Option<usize>,
    ) -> impl std::future::Future<Output = Result<Vec<String>>> + Send {
        // Default implementation: call find_by_property and truncate
        async move {
            let mut node_ids = self
                .find_by_property(scope, property_name, property_value, published_only)
                .await?;

            if let Some(lim) = limit {
                node_ids.truncate(lim);
            }

            Ok(node_ids)
        }
    }

    /// Count nodes matching a property value without materializing node data.
    ///
    /// This is optimized for COUNT(*) queries with property filters.
    fn count_by_property(
        &self,
        scope: StorageScope<'_>,
        property_name: &str,
        property_value: &models::nodes::properties::PropertyValue,
        published_only: bool,
    ) -> impl std::future::Future<Output = Result<usize>> + Send {
        // Default implementation: call find_by_property and return length
        async move {
            let node_ids = self
                .find_by_property(scope, property_name, property_value, published_only)
                .await?;
            Ok(node_ids.len())
        }
    }

    /// Scan all values for a property in sorted order.
    ///
    /// The default implementation returns an empty vec (backend does not support ordered scans).
    fn scan_property(
        &self,
        _scope: StorageScope<'_>,
        _property_name: &str,
        _published_only: bool,
        _ascending: bool,
        _limit: Option<usize>,
    ) -> impl std::future::Future<Output = Result<Vec<PropertyScanEntry>>> + Send {
        async { Ok(Vec::new()) }
    }

    /// Scan property values within a bounded range.
    ///
    /// This is optimized for range queries like `created_at > now()` or
    /// `updated_at BETWEEN x AND y`.
    ///
    /// The default implementation returns an empty vec (backend does not support range scans).
    fn scan_property_range(
        &self,
        _scope: StorageScope<'_>,
        _property_name: &str,
        _lower_bound: Option<(&models::nodes::properties::PropertyValue, bool)>,
        _upper_bound: Option<(&models::nodes::properties::PropertyValue, bool)>,
        _published_only: bool,
        _ascending: bool,
        _limit: Option<usize>,
    ) -> impl std::future::Future<Output = Result<Vec<PropertyScanEntry>>> + Send {
        async { Ok(Vec::new()) }
    }
}
