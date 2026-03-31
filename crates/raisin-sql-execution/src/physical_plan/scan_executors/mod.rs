// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Scan Executor Implementations.
//!
//! Implements physical scan operators that read data from RocksDB storage.
//! Each scan type uses a different access method optimized for specific query patterns.
//!
//! # Module Structure
//!
//! - `helpers` - Shared utility functions (locale resolution, translation, filter extraction)
//! - `node_to_row` - Node-to-Row conversion with projection and virtual columns
//! - `count_scan` - Optimized COUNT(*) operations
//! - `table_scan` - Full table DFS traversal
//! - `prefix_scan` - Path prefix scans (direct children or all descendants)
//! - `property_index_scan` - Property value lookups via property index
//! - `property_order_scan` - Ordered scans (ORDER BY with property index)
//! - `property_range_scan` - Range scans on property values
//! - `point_lookup` - O(1) path and node ID lookups
//! - `neighbors_scan` - Graph traversal via relation index
//! - `vector_scan` - k-NN search via HNSW index
//! - `spatial_scan` - Geospatial proximity and k-NN queries
//! - `compound_scan` - Multi-column compound index scans
//! - `reference_scan` - Reverse reference index scans

mod compound_scan;
mod count_scan;
mod helpers;
mod neighbors_scan;
mod node_to_row;
mod point_lookup;
mod prefix_scan;
mod property_index_scan;
mod property_order_scan;
mod property_range_scan;
mod reference_scan;
mod spatial_scan;
mod table_scan;
mod vector_scan;

use std::time::Duration;

// Re-export all public scan executor functions
pub use compound_scan::execute_compound_index_scan;
pub use count_scan::{execute_count_scan, execute_property_index_count_scan};
pub use neighbors_scan::execute_neighbors_scan;
pub(crate) use node_to_row::node_to_row;
pub use point_lookup::{execute_node_id_scan, execute_path_index_scan};
pub use prefix_scan::execute_prefix_scan;
pub use property_index_scan::execute_property_index_scan;
pub use property_order_scan::execute_property_order_scan;
pub use property_range_scan::execute_property_range_scan;
pub use reference_scan::execute_reference_index_scan;
pub use spatial_scan::{execute_spatial_distance_scan, execute_spatial_knn_scan};
pub use table_scan::execute_table_scan;
pub use vector_scan::execute_vector_scan;

/// Maximum time allowed for a scan operation before stopping.
/// Queries will return whatever results were found within this time.
const SCAN_TIME_LIMIT: Duration = Duration::from_secs(3);

/// Maximum number of nodes to check during a scan.
/// Acts as a hard ceiling to prevent CPU-intensive runaway scans.
const SCAN_COUNT_CEILING: usize = 200_000;

/// How often to check elapsed time during scanning.
/// We only check every N items to minimize Instant::now() syscall overhead.
const TIME_CHECK_INTERVAL: usize = 1000;
