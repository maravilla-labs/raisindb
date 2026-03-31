// SPDX-License-Identifier: BSL-1.1

//! Indexing framework for RaisinDB
//!
//! Provides pluggable indexing capabilities for efficient property lookups,
//! relationship queries, and other query patterns that would otherwise require
//! full workspace scans.
//!
//! # Full-Text Search
//!
//! The full-text search system uses Tantivy for language-aware indexing:
//! - `tantivy_engine` - Tantivy-based indexing engine
//! - `worker` - Background worker for processing indexing jobs
//! - `event_handler` - Event handler for automatic index updates

mod config;
mod manager;
mod plugin;
mod property_index;
mod query;

// Full-text search modules
pub mod event_handler;
pub mod management;
pub mod tantivy_engine;
pub mod worker;

pub use config::IndexCacheConfig;
pub use manager::IndexManager;
pub use plugin::IndexPlugin;
pub use property_index::PropertyIndexPlugin;
pub use query::IndexQuery;

// Re-export full-text search types
pub use event_handler::FullTextEventHandler;
pub use management::TantivyManagement;
pub use tantivy_engine::{BatchIndexContext, TantivyIndexingEngine};
pub use worker::{IndexerWorker, WorkerConfig};
