//! Index-based scan operator documentation and helpers.
//!
//! This module documents the index-driven scan variants of [`PhysicalPlan`].
//! These operators use specialized indexes for efficient data access.
//!
//! ## PropertyIndexScan
//!
//! Uses the `property_index` column family to find nodes by property value.
//! Optimal for: `WHERE properties->>'status' = 'published'`
//!
//! ## PropertyIndexCountScan
//!
//! Counts nodes matching a property value without deserializing node data.
//! 10-100x faster than PropertyIndexScan + HashAggregate for counting,
//! using only O(1) memory.
//!
//! ### Example Queries
//! ```sql
//! SELECT COUNT(*) FROM nodes WHERE properties->>'status' = 'published'
//! SELECT COUNT(*) FROM nodes WHERE node_type = 'Post'
//! ```
//!
//! ## PropertyOrderScan
//!
//! Streams nodes ordered by a pseudo-property (e.g., `created_at`) directly
//! from the property index. Avoids a separate sort step.
//!
//! ## CompoundIndexScan
//!
//! Scans a compound index that combines multiple property columns for efficient
//! multi-column queries with optional ordering.
//!
//! ### Example Query
//! ```sql
//! SELECT * FROM nodes
//! WHERE node_type = 'news:Article'
//!   AND properties->>'category' = 'business'
//! ORDER BY created_at DESC
//! LIMIT 10
//! ```
//!
//! With a compound index on `(node_type, category, created_at DESC)`, this
//! executes as a single prefix scan in O(LIMIT) time.
//!
//! ## PropertyRangeScan
//!
//! Scans the property index for values within a bounded range. Optimal for
//! range queries like:
//! - `WHERE created_at > now()`
//! - `WHERE updated_at < '2024-01-01'`
//! - `WHERE created_at > X AND created_at < Y`
//!
//! Uses RocksDB seek for O(k) performance where k is matching rows.
//!
//! ## PathIndexScan
//!
//! Uses the `path_index` column family to find a node by exact path.
//! O(1) lookup time - much faster than PrefixScan or TableScan for exact matches.
//!
//! ## NodeIdScan
//!
//! Uses the NODES column family to fetch a node directly by its ID.
//! O(1) lookup time - fastest possible access method for known node IDs.
//!
//! ## FullTextScan
//!
//! Queries the Tantivy full-text index and returns matching nodes ranked by
//! relevance. Used for PostgreSQL-style full-text search:
//! ```sql
//! WHERE to_tsvector('english', content) @@ to_tsquery('english', 'query')
//! ```
//!
//! ## NeighborsScan
//!
//! Queries the relation index to find connected nodes via graph edges.
//! Used for graph traversal queries.
//!
//! ## SpatialDistanceScan
//!
//! Queries the `spatial_index` column family to find nodes within a given
//! distance of a point. Uses geohash-based indexing for PostGIS-compatible
//! `ST_DWithin` queries.
//!
//! ### Performance
//! - Uses geohash cell expansion for candidate filtering
//! - Final distance filtering with Haversine formula
//! - O(k) where k is number of matching nodes
//!
//! ## SpatialKnnScan
//!
//! Finds the k closest nodes to a query point using the spatial index.
//! Uses progressive ring expansion for efficient nearest neighbor search.
//!
//! ### Performance
//! - Uses adaptive geohash precision based on data density
//! - Ring expansion to find minimum required candidates
//! - Final exact distance sorting with Haversine formula
//!
//! ## ReferenceIndexScan
//!
//! Uses the `reference_index` column family (`ref_rev` prefix) to efficiently
//! find all nodes that reference a specific target node.
//!
//! ### Performance
//! - O(k) where k is the number of nodes referencing the target
//! - Uses RocksDB prefix iterator on `ref_rev` CF
//! - Much faster than full table scan with reference property check
//!
//! ### Example
//! ```sql
//! SELECT * FROM social
//! WHERE REFERENCES('social:/demonews/tags/tech-stack/rust')
//! ```
//!
//! ## VectorScan
//!
//! Performs k-nearest neighbor (k-NN) search for vector embeddings using
//! HNSW index. O(log n) approximate nearest neighbor.
//!
//! ### Example
//! ```sql
//! SELECT * FROM nodes
//! ORDER BY (embedding <=> EMBEDDING('query'))
//! LIMIT k
//! ```
//!
//! ## CTEScan
//!
//! References a CTE that was previously materialized by a WithCTE operator.
//! Reads from either in-memory cache or disk-spilled files transparently.
//!
//! ### Performance
//! - In-memory CTEs: Sub-millisecond scan times
//! - Disk-spilled CTEs: ~200MB/s sequential read (benefits from OS page cache)

use super::PhysicalPlan;

impl PhysicalPlan {
    /// Returns true if this is an index-based scan operator.
    pub fn is_index_scan(&self) -> bool {
        matches!(
            self,
            PhysicalPlan::PropertyIndexScan { .. }
                | PhysicalPlan::PropertyIndexCountScan { .. }
                | PhysicalPlan::PropertyOrderScan { .. }
                | PhysicalPlan::CompoundIndexScan { .. }
                | PhysicalPlan::PropertyRangeScan { .. }
                | PhysicalPlan::PathIndexScan { .. }
                | PhysicalPlan::NodeIdScan { .. }
                | PhysicalPlan::FullTextScan { .. }
                | PhysicalPlan::NeighborsScan { .. }
                | PhysicalPlan::SpatialDistanceScan { .. }
                | PhysicalPlan::SpatialKnnScan { .. }
                | PhysicalPlan::ReferenceIndexScan { .. }
                | PhysicalPlan::VectorScan { .. }
                | PhysicalPlan::CTEScan { .. }
        )
    }
}
