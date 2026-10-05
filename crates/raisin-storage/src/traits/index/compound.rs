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

impl CompoundColumnValue {
    /// The ONE encoding of a `String` column's value, shared by the index
    /// writers and the SQL executor's equality prefix.
    ///
    /// A string the property decoder reads back as a DATE is stored as that
    /// date: `PropertyValue` is untagged, and any string
    /// `DateTime::parse_from_rfc3339` accepts decodes as `Date`. A writer
    /// deriving entries from the in-memory node saw the string, one deriving
    /// them from the stored version (a baseline, a rebuild) saw the date — two
    /// different keys for one value, so an update could never tombstone what
    /// the insert wrote, and a rebuild left such nodes out of the index. Both
    /// spellings therefore encode as the date's canonical RFC3339 text.
    pub fn text(value: &str) -> Self {
        match chrono::DateTime::parse_from_rfc3339(value) {
            Ok(dt) => Self::date_text(dt.with_timezone(&chrono::Utc)),
            Err(_) => Self::String(value.to_string()),
        }
    }

    /// A `Date` value in a `String` column: its canonical RFC3339 text (UTC,
    /// `Z`, the shortest exact fraction) — see [`Self::text`].
    pub fn date_text(value: chrono::DateTime<chrono::Utc>) -> Self {
        Self::String(value.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
    }
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
    /// Each `(tuple, node)` is decided by its newest entry at or below
    /// `max_revision` (`None` = every stored entry): a read at a past revision
    /// sees the index as it stood then.
    ///
    /// # Returns
    /// Vector of (node_id, optional_timestamp) entries in index order.
    #[allow(clippy::too_many_arguments)]
    fn scan_compound_index(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        equality_values: &[CompoundColumnValue],
        published_only: bool,
        ascending: bool,
        limit: Option<usize>,
        max_revision: Option<&HLC>,
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
