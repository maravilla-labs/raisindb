// SPDX-License-Identifier: BSL-1.1

//! Database-level management operations.
//!
//! These handlers manage repository-specific indexes (fulltext, vector,
//! RocksDB property/reference/child_order) and relation integrity checks.

mod fulltext;
mod reindex;
mod relations;
mod spatial;
mod stubs;
pub mod types;
mod vector;
mod vector_embeddings;

// Re-export all handler functions to preserve `crate::handlers::management::database::*` paths.
#[cfg(feature = "storage-rocksdb")]
pub use fulltext::{
    clear_fulltext_errors, get_fulltext_errors, get_fulltext_health, optimize_fulltext_index,
    purge_fulltext_index, rebuild_fulltext_index, reconcile_fulltext_index, verify_fulltext_index,
};

#[cfg(feature = "storage-rocksdb")]
pub use vector::{
    get_vector_health, optimize_vector_index, rebuild_vector_index, restore_vector_index,
    verify_vector_index,
};

#[cfg(feature = "storage-rocksdb")]
pub use vector_embeddings::regenerate_vector_embeddings;

#[cfg(feature = "storage-rocksdb")]
pub use reindex::reindex_start;

#[cfg(feature = "storage-rocksdb")]
pub use relations::{repair_relation_integrity, verify_relation_integrity};

#[cfg(feature = "storage-rocksdb")]
pub use spatial::{
    get_spatial_config, get_spatial_health, put_spatial_config, rebuild_spatial_index,
    verify_spatial_index,
};

// Non-rocksdb stubs
#[cfg(not(feature = "storage-rocksdb"))]
pub use stubs::*;
