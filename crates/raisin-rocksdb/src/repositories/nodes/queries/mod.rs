//! Query operations for nodes (filtering, searching, listing)
//!
//! This module is organized into several sub-modules:
//! - `listing` - List nodes by type, parent, or root
//! - `lookup` - Get and delete nodes by path
//! - `property` - Query and update node properties
//! - `deep` - Deep hierarchical queries
//! - `validation` - Validation helpers for tree operations
//! - `tree_ops` - Move and rename operations
//! - `copy` - Copy node and tree operations
//! - `scanning` - Descendant scanning and bulk operations
//! - `translations` - Translation key builders and helpers

mod copy;
mod deep;
mod listing;
mod lookup;
mod property;
mod scanning;
mod translations;
mod tree_ops;
mod validation;

// Re-export all public functions from each module
// The functions maintain their original visibility modifiers
// pub(in super::super) means accessible from nodes/mod.rs
// pub(crate) means accessible from anywhere in the crate
