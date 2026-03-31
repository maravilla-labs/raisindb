//! Index Catalog
//!
//! Provides an abstraction for querying available indexes in the storage engine.
//! This allows the physical planner to make intelligent scan selection decisions
//! based on what indexes are actually available.
//!
//! # Architecture
//!
//! The catalog pattern allows different storage backends to advertise their
//! capabilities without the physical planner needing to know storage-specific details.
//!
//! # Example
//!
//! ```rust,ignore
//! use raisin_sql::physical_plan::{IndexCatalog, RocksDBIndexCatalog};
//!
//! // RocksDB always has path_index and property_index CFs
//! let catalog = RocksDBIndexCatalog::new();
//! assert!(catalog.has_path_index());
//! assert!(catalog.has_property_index());
//!
//! // Full-text index depends on Tantivy configuration
//! let catalog_no_fts = RocksDBIndexCatalog::without_fulltext();
//! assert!(!catalog_no_fts.has_fulltext_index());
//! ```

/// Trait for querying available indexes in the storage engine
///
/// Implementations should return true only for indexes that are actually
/// available and ready to use. The physical planner uses this to select
/// the optimal scan method.
pub trait IndexCatalog: Send + Sync {
    /// Check if path_index column family is available
    ///
    /// The path_index supports efficient prefix scans for hierarchy queries:
    /// - `PATH_STARTS_WITH(path, '/content/')`
    /// - `PARENT(path) = '/content'`
    ///
    /// Key format: `{tenant}\0{repo}\0{branch}\0{workspace}\0path\0{path}\0{~revision}`
    fn has_path_index(&self) -> bool;

    /// Check if property_index column family is available
    ///
    /// The property_index supports efficient property lookups:
    /// - `properties->>'status' = 'published'`
    /// - `__node_type = 'Document'`
    ///
    /// Key format: `{tenant}\0{repo}\0{branch}\0{workspace}\0prop{_pub}\0{property_name}\0{value_hash}\0{~revision}\0{node_id}`
    fn has_property_index(&self) -> bool;

    /// Check if Tantivy full-text index is available
    ///
    /// The full-text index supports PostgreSQL-style text search:
    /// - `to_tsvector('english', content) @@ to_tsquery('english', 'query')`
    fn has_fulltext_index(&self) -> bool;

    /// Check if spatial_index column family is available
    ///
    /// The spatial_index supports geohash-based proximity queries:
    /// - `ST_DWithin(properties->>'location', ST_Point(-122.4, 37.8), 1000)`
    ///
    /// Key format: `{tenant}\0{repo}\0{branch}\0{workspace}\0geo\0{property}\0{geohash}\0{~revision}\0{node_id}`
    fn has_spatial_index(&self) -> bool;

    /// Check if compound_index column family is available
    ///
    /// The compound_index supports efficient multi-column queries with ordering:
    /// - `WHERE node_type = 'Article' AND category = 'business' ORDER BY created_at DESC`
    ///
    /// Key format: `{tenant}\0{repo}\0{branch}\0{workspace}\0cidx\0{index_name}\0{col1}\0{col2}\0...\0{~revision}\0{node_id}`
    fn has_compound_index(&self) -> bool {
        true // Available by default in RocksDB
    }

    /// Find a compound index matching the given query pattern
    ///
    /// Returns the index name and column configuration if a matching compound index
    /// exists for the given node_type and filter/order by columns.
    ///
    /// # Arguments
    /// * `node_type` - The node type to check for compound indexes
    /// * `equality_columns` - Columns that have equality predicates (e.g., category = 'business')
    /// * `order_column` - The column used in ORDER BY (e.g., created_at)
    /// * `ascending` - Whether ORDER BY is ascending
    ///
    /// # Returns
    /// Some((index_name, column_count)) if a matching index exists, None otherwise.
    ///
    /// # Note
    /// This requires runtime access to NodeType definitions. The default implementation
    /// returns None. Concrete implementations should wire up NodeType repository access.
    fn find_compound_index(
        &self,
        _node_type: &str,
        _equality_columns: &[&str],
        _order_column: &str,
        _ascending: bool,
    ) -> Option<(String, usize)> {
        // Default: No compound index matching (requires NodeType access to implement)
        None
    }

    /// Get a list of all available index names (for debugging/EXPLAIN)
    fn available_indexes(&self) -> Vec<String> {
        let mut indexes = Vec::new();
        if self.has_path_index() {
            indexes.push("path_index".to_string());
        }
        if self.has_property_index() {
            indexes.push("property_index".to_string());
        }
        if self.has_fulltext_index() {
            indexes.push("fulltext_index".to_string());
        }
        if self.has_spatial_index() {
            indexes.push("spatial_index".to_string());
        }
        if self.has_compound_index() {
            indexes.push("compound_index".to_string());
        }
        indexes
    }
}

/// RocksDB-specific index catalog
///
/// RocksDB storage in RaisinDB has the following column families:
/// 1. **path_index** - Always available (core hierarchy support)
/// 2. **property_index** - Always available (core property lookups)
/// 3. **fulltext_index** - Depends on Tantivy indexer configuration
///
/// # Notes
///
/// - The path_index and property_index are always present in RocksDB
/// - Full-text search requires the Tantivy indexer to be enabled
/// - This catalog does NOT check if indexes exist at runtime - it represents
///   the expected configuration
#[derive(Debug, Clone)]
pub struct RocksDBIndexCatalog {
    /// Whether Tantivy full-text index is enabled
    has_fulltext: bool,
}

impl RocksDBIndexCatalog {
    /// Create a new catalog with all indexes enabled
    ///
    /// This is the default configuration for RaisinDB with Tantivy.
    pub fn new() -> Self {
        Self { has_fulltext: true }
    }

    /// Create a catalog without full-text search
    ///
    /// Use this when the Tantivy indexer is not configured or disabled.
    pub fn without_fulltext() -> Self {
        Self {
            has_fulltext: false,
        }
    }

    /// Create a catalog with custom full-text configuration
    pub fn with_fulltext(enabled: bool) -> Self {
        Self {
            has_fulltext: enabled,
        }
    }
}

impl Default for RocksDBIndexCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexCatalog for RocksDBIndexCatalog {
    fn has_path_index(&self) -> bool {
        // path_index CF is always present in RocksDB
        true
    }

    fn has_property_index(&self) -> bool {
        // property_index CF is always present in RocksDB
        true
    }

    fn has_fulltext_index(&self) -> bool {
        self.has_fulltext
    }

    fn has_spatial_index(&self) -> bool {
        // spatial_index CF is always present in RocksDB
        // It uses geohash-based indexing for proximity queries
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rocksdb_catalog_default() {
        let catalog = RocksDBIndexCatalog::new();
        assert!(catalog.has_path_index());
        assert!(catalog.has_property_index());
        assert!(catalog.has_fulltext_index());
        assert!(catalog.has_spatial_index());
        assert!(catalog.has_compound_index());

        let indexes = catalog.available_indexes();
        assert_eq!(indexes.len(), 5);
        assert!(indexes.contains(&"path_index".to_string()));
        assert!(indexes.contains(&"property_index".to_string()));
        assert!(indexes.contains(&"fulltext_index".to_string()));
        assert!(indexes.contains(&"spatial_index".to_string()));
        assert!(indexes.contains(&"compound_index".to_string()));
    }

    #[test]
    fn test_rocksdb_catalog_without_fulltext() {
        let catalog = RocksDBIndexCatalog::without_fulltext();
        assert!(catalog.has_path_index());
        assert!(catalog.has_property_index());
        assert!(!catalog.has_fulltext_index());
        assert!(catalog.has_spatial_index());
        assert!(catalog.has_compound_index());

        let indexes = catalog.available_indexes();
        assert_eq!(indexes.len(), 4);
        assert!(indexes.contains(&"path_index".to_string()));
        assert!(indexes.contains(&"property_index".to_string()));
        assert!(indexes.contains(&"spatial_index".to_string()));
        assert!(indexes.contains(&"compound_index".to_string()));
    }

    #[test]
    fn test_rocksdb_catalog_custom_fulltext() {
        let catalog = RocksDBIndexCatalog::with_fulltext(false);
        assert!(!catalog.has_fulltext_index());

        let catalog = RocksDBIndexCatalog::with_fulltext(true);
        assert!(catalog.has_fulltext_index());
    }

    #[test]
    fn test_default_trait() {
        let catalog = RocksDBIndexCatalog::default();
        assert!(catalog.has_fulltext_index());
    }
}
