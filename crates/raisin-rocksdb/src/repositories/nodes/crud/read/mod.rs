//! Node retrieval operations
//!
//! This module contains all read operations for nodes:
//! - get_impl: Get node at HEAD revision
//! - get_at_revision_impl: Get node at specific revision (time-travel)
//! - list_all_impl: List all nodes in workspace
//! - count_all_impl: Count all nodes in workspace
//!
//! # StorageNode Optimization
//!
//! Nodes are stored as `StorageNode` which excludes the `path` field from the blob.
//! The path is materialized from the NODE_PATH index during reads.
//! This enables O(1) move operations (only root node blob + path indexes need updating).

mod get_operations;
mod list_operations;
mod path_materialization;
