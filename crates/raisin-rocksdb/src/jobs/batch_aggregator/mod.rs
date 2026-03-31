//! Batch aggregation service for fulltext indexing
//!
//! This module provides a service that aggregates individual fulltext indexing
//! operations into batch jobs to reduce Tantivy commit overhead during bulk
//! import operations.
//!
//! # How it works
//!
//! 1. Individual node events call `queue()` to add operations
//! 2. Operations are grouped by index key (tenant/repo/branch)
//! 3. When a threshold is reached (count or time), operations are flushed
//!    as a single `FulltextBatchIndex` job
//! 4. A background task periodically flushes expired batches
//!
//! # Performance
//!
//! For 1M nodes:
//! - Without aggregation: 1M individual jobs = 1M Tantivy commits = ~14+ hours
//! - With aggregation: ~1K batch jobs = ~1K Tantivy commits = ~15-30 minutes

mod core;
#[cfg(test)]
mod tests;

pub use self::core::{BatchAggregatorConfig, BatchIndexAggregator};
