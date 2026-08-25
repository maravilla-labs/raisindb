//! Index Catalog
//!
//! Provides an abstraction for querying available indexes in the storage engine.
//! This allows the physical planner to make intelligent scan selection decisions
//! based on what indexes are actually available.
//!
//! # Architecture
//!
//! The catalog pattern allows different storage backends to advertise their
//! capabilities without the physical planner needing to know storage-specific
//! details.
//!
//! # The spatial index is different, and why
//!
//! Every other index here is a *capability* question ("is the CF present, is
//! Tantivy configured"), answerable with a `bool` that is constant for the
//! process. The spatial index is a *state* question, per workspace and per
//! property: the entries only exist if something wrote them, and a workspace
//! upgraded from a release without spatial indexing has none at all.
//!
//! Answering it with a hardcoded `true` — which is what this file did — combined
//! with a planner that removed `ST_DWITHIN` from the residual filter produced the
//! worst failure mode a database has: **zero rows, silently, with no error and no
//! fallback**. `spatial_index_availability` replaces that with a scoped query
//! that **fails closed**: the default is `NotBuilt`, so a backend that has not
//! wired up a state source gets a correct (slower) full scan rather than a fast
//! wrong answer.
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

mod spatial_availability;

pub use spatial_availability::{
    bucket_property, explain_reason, radius_is_covered, SpatialAvailability, SpatialStateSource,
    GEOHASH_CELL_RADIUS_METERS,
};

use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_storage::compound::{CompoundAvailability, CompoundStateSource};
use std::fmt;
use std::sync::Arc;

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

    /// Whether the spatial index for this scope's `workspace`.`property` may be
    /// used to drive a scan, and on what terms.
    ///
    /// # This method must fail closed
    ///
    /// The default returns [`SpatialAvailability::NotBuilt`]. That is deliberate
    /// and load-bearing: the presence of the `spatial_index` column family says
    /// nothing about whether it holds entries for this workspace and property,
    /// and a catalog with no state source cannot know. Answering "probably yes"
    /// would reintroduce the silent-empty bug this signature exists to remove.
    ///
    /// Callers must treat a non-`Ready` answer as "do not use the index, keep the
    /// predicate as a row-level filter, and say so in EXPLAIN".
    fn spatial_index_availability(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _property: &str,
    ) -> SpatialAvailability {
        SpatialAvailability::NotBuilt
    }

    /// Check if compound_index column family is available
    ///
    /// The compound_index supports efficient multi-column queries with ordering:
    /// - `WHERE node_type = 'Article' AND category = 'business' ORDER BY created_at DESC`
    ///
    /// Key format: `{tenant}\0{repo}\0{branch}\0{workspace}\0cidx\0{index_name}\0{col1}\0{col2}\0...\0{~revision}\0{node_id}`
    /// Whether the backend has a compound-index column family at all.
    ///
    /// Cosmetic — `available_indexes()` prints it. This is NOT a planning
    /// signal and must never become one: the CF existing says nothing about
    /// whether a particular index holds entries. Use
    /// [`Self::compound_index_availability`] for that.
    fn has_compound_index(&self) -> bool {
        true // Available by default in RocksDB
    }

    /// What one compound index can actually answer, for the declaration
    /// currently in force.
    ///
    /// # This method must fail closed
    ///
    /// The default returns [`CompoundAvailability::NotBuilt`], for the same
    /// reason `spatial_index_availability` does. A compound index name
    /// addresses a workspace-global keyspace with no node type in the key, so a
    /// changed declaration writes new-layout entries alongside old-layout ones.
    /// Trusting a declaration means building a scan prefix for a layout the
    /// stored bytes may not share — and because the planner STRIPS the matched
    /// equality predicates from the residual filter, the result is missing rows
    /// rather than a slow query.
    fn compound_index_availability(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _definition: &CompoundIndexDefinition,
    ) -> CompoundAvailability {
        CompoundAvailability::NotBuilt
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
    ///
    /// `spatial_index` is deliberately absent: availability is per workspace and
    /// per property, so there is no honest process-wide answer. Ask
    /// [`Self::spatial_index_availability`] instead.
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
/// 4. **spatial_index** - CF always present, but *populated* per workspace and
///    property; see [`Self::with_spatial_state`].
///
/// # Notes
///
/// - The path_index and property_index are always present in RocksDB
/// - Full-text search requires the Tantivy indexer to be enabled
/// - Apart from the spatial index, this catalog does NOT check whether indexes
///   exist at runtime — it represents the expected configuration
#[derive(Clone)]
pub struct RocksDBIndexCatalog {
    /// Whether Tantivy full-text index is enabled
    has_fulltext: bool,
    /// Source of per-(workspace, property) spatial index build state.
    ///
    /// `None` means "unknown", which resolves to
    /// [`SpatialAvailability::NotBuilt`] — never to "assume it works".
    spatial_state: Option<Arc<dyn SpatialStateSource>>,
    /// Same contract for compound indexes.
    compound_state: Option<Arc<dyn CompoundStateSource>>,
}

impl fmt::Debug for RocksDBIndexCatalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RocksDBIndexCatalog")
            .field("has_fulltext", &self.has_fulltext)
            .field("spatial_state", &self.spatial_state.is_some())
            .field("compound_state", &self.compound_state.is_some())
            .finish()
    }
}

impl RocksDBIndexCatalog {
    /// Create a new catalog with all *capability* indexes enabled.
    ///
    /// The spatial index reports `NotBuilt` until a state source is attached with
    /// [`Self::with_spatial_state`].
    pub fn new() -> Self {
        Self {
            has_fulltext: true,
            spatial_state: None,
            compound_state: None,
        }
    }

    /// Create a catalog without full-text search
    ///
    /// Use this when the Tantivy indexer is not configured or disabled.
    pub fn without_fulltext() -> Self {
        Self {
            has_fulltext: false,
            spatial_state: None,
            compound_state: None,
        }
    }

    /// Create a catalog with custom full-text configuration
    pub fn with_fulltext(enabled: bool) -> Self {
        Self {
            has_fulltext: enabled,
            spatial_state: None,
            compound_state: None,
        }
    }

    /// Attach the source of per-(workspace, property) spatial index build state.
    ///
    /// Without this, every spatial predicate falls back to a row-level filter on
    /// an ordinary scan — correct, slower, and loudly reported in EXPLAIN.
    pub fn with_spatial_state(mut self, source: Arc<dyn SpatialStateSource>) -> Self {
        self.spatial_state = Some(source);
        self
    }

    /// Attach a state source if the backend has one.
    ///
    /// Convenience for `Storage::spatial_state()`, which returns `None` for
    /// backends that cannot report build state. `None` keeps the fail-closed
    /// default.
    pub fn with_optional_spatial_state(
        mut self,
        source: Option<Arc<dyn SpatialStateSource>>,
    ) -> Self {
        self.spatial_state = source;
        self
    }

    /// Attach the source of per-(workspace, index) compound build state.
    ///
    /// `None` keeps the fail-closed default: every compound index reads as
    /// `NotBuilt` and the planner declines all of them. That is the correct
    /// answer for a backend that cannot report build state.
    pub fn with_optional_compound_state(
        mut self,
        source: Option<Arc<dyn CompoundStateSource>>,
    ) -> Self {
        self.compound_state = source;
        self
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

    fn spatial_index_availability(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
    ) -> SpatialAvailability {
        match &self.spatial_state {
            Some(source) => {
                source.spatial_availability(tenant_id, repo_id, branch, workspace, property)
            }
            // FAIL CLOSED. The CF exists; that is not the question.
            None => SpatialAvailability::NotBuilt,
        }
    }

    fn compound_index_availability(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        definition: &CompoundIndexDefinition,
    ) -> CompoundAvailability {
        match &self.compound_state {
            Some(source) => {
                source.compound_availability(tenant_id, repo_id, branch, workspace, definition)
            }
            // FAIL CLOSED. The CF exists; that was never the question.
            None => CompoundAvailability::NotBuilt,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state source that reports one fixed answer, for the fail-closed tests.
    #[derive(Debug)]
    struct FixedState(SpatialAvailability);

    impl SpatialStateSource for FixedState {
        fn spatial_availability(
            &self,
            _tenant_id: &str,
            _repo_id: &str,
            _branch: &str,
            _workspace: &str,
            _property: &str,
        ) -> SpatialAvailability {
            self.0.clone()
        }
    }

    fn availability(catalog: &RocksDBIndexCatalog) -> SpatialAvailability {
        catalog.spatial_index_availability("default", "default", "main", "shops", "location")
    }

    #[test]
    fn test_rocksdb_catalog_default() {
        let catalog = RocksDBIndexCatalog::new();
        assert!(catalog.has_path_index());
        assert!(catalog.has_property_index());
        assert!(catalog.has_fulltext_index());
        assert!(catalog.has_compound_index());

        let indexes = catalog.available_indexes();
        assert_eq!(indexes.len(), 4);
        assert!(indexes.contains(&"path_index".to_string()));
        assert!(indexes.contains(&"property_index".to_string()));
        assert!(indexes.contains(&"fulltext_index".to_string()));
        assert!(indexes.contains(&"compound_index".to_string()));
        // Per-property state, so no process-wide entry.
        assert!(!indexes.contains(&"spatial_index".to_string()));
    }

    #[test]
    fn test_rocksdb_catalog_without_fulltext() {
        let catalog = RocksDBIndexCatalog::without_fulltext();
        assert!(catalog.has_path_index());
        assert!(catalog.has_property_index());
        assert!(!catalog.has_fulltext_index());
        assert!(catalog.has_compound_index());

        let indexes = catalog.available_indexes();
        assert_eq!(indexes.len(), 3);
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

    /// The regression this whole signature exists for: a catalog with no state
    /// source must NOT claim the spatial index is usable.
    #[test]
    fn spatial_availability_fails_closed_without_a_state_source() {
        let catalog = RocksDBIndexCatalog::new();
        assert!(matches!(
            availability(&catalog),
            SpatialAvailability::NotBuilt
        ));
        assert!(!availability(&catalog).is_ready());
    }

    #[test]
    fn spatial_availability_delegates_to_the_state_source() {
        let ready = SpatialAvailability::Ready {
            precisions: vec![11, 10, 9, 8, 7, 6, 4, 2],
            built_through: raisin_hlc::HLC::now(),
            bucket_property: Some("floor".to_string()),
        };
        let catalog = RocksDBIndexCatalog::new().with_spatial_state(Arc::new(FixedState(ready)));
        assert!(availability(&catalog).is_ready());
        assert_eq!(
            availability(&catalog).precisions(),
            &[11, 10, 9, 8, 7, 6, 4, 2]
        );

        let unusable = SpatialAvailability::Unusable("state record is corrupt".to_string());
        let catalog = RocksDBIndexCatalog::new().with_spatial_state(Arc::new(FixedState(unusable)));
        assert!(!availability(&catalog).is_ready());
    }
}
