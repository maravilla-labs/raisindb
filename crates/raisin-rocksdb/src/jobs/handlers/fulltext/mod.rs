//! Fulltext indexing job handler
//!
//! This module handles fulltext indexing operations for nodes, including
//! adding/updating nodes in the index, deleting nodes, and copying indexes
//! for new branches.

mod batch;
mod error_counter;
mod handler;

pub use error_counter::{
    ErrorCounterKey, FulltextErrorCounter, FulltextErrorKind, FulltextErrorStats,
};
pub use handler::FulltextJobHandler;
