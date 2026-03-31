// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Compound index repository trait for multi-column queries with ORDER BY

use raisin_error::Result;
use raisin_hlc::HLC;

use crate::scope::StorageScope;

/// Entry returned from a compound index scan
#[derive(Debug, Clone)]
pub struct CompoundIndexScanEntry {
    /// Node ID
    pub node_id: String,
    /// Optional timestamp value (for ORDER BY timestamp queries)
    pub timestamp: Option<i64>,
}

/// Column value type for compound index keys.
///
/// This enum represents the different types of values that can be
/// stored in compound index columns. Each type has specific encoding
/// rules to ensure proper sort order.
#[derive(Debug, Clone, PartialEq)]
pub enum CompoundColumnValue {
    /// String column value (node_type, category, etc.)
    String(std::string::String),
    /// Integer column value
    Integer(i64),
    /// Timestamp in descending order (most recent first)
    /// Encoded as bitwise NOT of microseconds for proper sort order
    TimestampDesc(i64),
    /// Timestamp in ascending order (oldest first)
    TimestampAsc(i64),
    /// Boolean column value
    Boolean(bool),
}

/// Compound index repository for multi-column queries with ORDER BY.
///
/// Compound indexes enable efficient execution of queries like:
/// ```sql
/// SELECT * FROM nodes
/// WHERE node_type = 'news:Article'
///   AND properties->>'category' = 'business'
/// ORDER BY created_at DESC
/// LIMIT 10
/// ```
///
/// By combining multiple equality columns with a trailing timestamp column,
/// these queries execute in O(LIMIT) time instead of scanning all matching nodes.
///
/// # Scoped Architecture
///
/// All methods take a `StorageScope` (tenant + repo + branch + workspace).
///
/// # Key Format
///
/// ```text
/// {tenant}\0{repo}\0{branch}\0{workspace}\0cidx\0{index_name}\0{col1_value}\0{col2_value}\0...\0{timestamp}\0{revision}\0{node_id}
/// ```
pub trait CompoundIndexRepository: Send + Sync {
    /// Index a node in a compound index.
    ///
    /// Called when a node is created or updated. The caller must extract
    /// the relevant column values from the node's properties.
    fn index_compound(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        column_values: &[CompoundColumnValue],
        revision: &HLC,
        node_id: &str,
        is_published: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Remove a node from a compound index.
    ///
    /// Called when a node is deleted. Removes entries from both
    /// draft and published spaces if they exist.
    fn unindex_compound(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        column_values: &[CompoundColumnValue],
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Scan a compound index with equality prefix.
    ///
    /// Scans the index for entries matching all provided equality column values.
    /// Results are returned in index order (sorted by trailing timestamp column).
    ///
    /// # Returns
    /// Vector of (node_id, optional_timestamp) entries in index order.
    fn scan_compound_index(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        equality_values: &[CompoundColumnValue],
        published_only: bool,
        ascending: bool,
        limit: Option<usize>,
    ) -> impl std::future::Future<Output = Result<Vec<CompoundIndexScanEntry>>> + Send;

    /// Remove all compound index entries for a node across all indexes.
    ///
    /// Called when a node is fully deleted. This scans all compound indexes
    /// in the workspace and removes any entries for this node.
    fn remove_all_compound_indexes_for_node(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}
