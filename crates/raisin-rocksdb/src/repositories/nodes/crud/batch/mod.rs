//! Batch operation helpers for node CRUD
//!
//! This module contains helper functions for building WriteBatch operations:
//! - Adding nodes to WriteBatch with all necessary indexes
//! - Adding ordered children index entries
//! - Optimized fast-path for new node creation

mod has_children;
mod ordered_children;
mod write_batch;
