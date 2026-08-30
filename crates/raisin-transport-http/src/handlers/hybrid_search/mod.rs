// SPDX-License-Identifier: BSL-1.1

//! Hybrid search over HTTP.
//!
//! This module is a THIN ADAPTER over
//! `raisin_sql_execution::physical_plan::search`, the same implementation
//! `SELECT ... FROM HYBRID_SEARCH(...)` uses. It holds request/response types
//! and nothing else.
//!
//! It used to hold a full second implementation -- `fulltext.rs`, `vector.rs`
//! and an `rrf.rs` whose fusion map was keyed on `node_id` alone, the very bug
//! the SQL side fixed by keying on `(workspace_id, node_id)`. None of those
//! three files mentioned `auth`, so the endpoint applied no row-level security
//! whatsoever. All three are deleted; do not reintroduce a leg or a fusion here.
//!
//! # RRF
//!
//! For each document d and rank lists R1..Rn, `RRF(d) = sum 1 / (k + rank_i(d))`
//! with k fixed at 60. The `k` query parameter is accepted, ignored and
//! deprecated -- see the handler.

mod handler;
mod types;

// Re-export public API
#[cfg(feature = "storage-rocksdb")]
pub use handler::hybrid_search;
pub use types::{HybridSearchQuery, HybridSearchResponse, HybridSearchResult};
