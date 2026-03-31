//! Fulltext indexing job handler
//!
//! This module handles fulltext indexing operations for nodes, including
//! adding/updating nodes in the index, deleting nodes, and copying indexes
//! for new branches.

mod batch;
mod handler;

pub use handler::FulltextJobHandler;
