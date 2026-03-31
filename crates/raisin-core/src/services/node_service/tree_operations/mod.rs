//! Tree operation methods for NodeService.
//!
//! This module contains operations for managing the hierarchical tree structure
//! of nodes:
//! - Moving nodes (change parent)
//! - Renaming nodes
//! - Reordering children
//! - Deep tree queries (nested, flat, array formats)
//! - Tree navigation helpers

mod deep_queries;
mod deep_queries_revision;
mod list_children;
mod move_rename;
mod reorder;
mod tree_navigation;
