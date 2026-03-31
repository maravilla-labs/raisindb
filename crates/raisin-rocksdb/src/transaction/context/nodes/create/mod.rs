//! Node creation operations
//!
//! This module contains the implementation of node creation operations for transactions:
//! - `put_node`: Create or update a node (validates and handles both cases)
//! - `add_node`: Optimized path for new nodes (validates as CREATE only)
//!
//! # Key Features
//!
//! ## Parent Normalization
//!
//! The parent field is normalized from the path before saving. Parent is NEVER null:
//! - Root-level nodes have parent = "/"
//! - Other nodes have parent = parent's name
//!
//! ## Validation
//!
//! Uses NodeRepository validation helpers:
//! - CREATE: Validates parent allows child, workspace allows type
//! - UPDATE: Validates updates are allowed (with type change support for migrations)
//!
//! ## Indexes
//!
//! Updates all indexes atomically:
//! - NODES: Node data with versioned key
//! - PATH_INDEX: Path -> node_id mapping (with tombstone for old path on moves)
//! - PROPERTY_INDEX: Property value indexes for queries
//! - REFERENCE_INDEX: Forward and reverse reference indexes
//! - ORDERED_CHILDREN: Fractional index for ordered children
//!
//! ## Change Tracking
//!
//! Tracks changes for:
//! - Revision snapshot creation (async background job)
//! - NodeEvent emission (WebSocket notifications)
//! - CRDT replication (distributed sync)
//!
//! # Module Structure
//!
//! The module is organized into focused submodules:
//!
//! - `core`: Main public functions (put_node, add_node)
//! - `metadata`: Metadata extraction and normalization
//! - `validation`: Validation logic for create and update operations
//! - `cache`: Read cache management for read-your-writes semantics
//! - `storage`: Storage operations (write node, path index)
//! - `indexing`: Property and reference indexing
//! - `ordering`: ORDERED_CHILDREN index management
//! - `tracking`: Change tracking for replication and events
//! - `references`: Path-based reference resolution

mod cache;
mod core;
mod indexing;
mod metadata;
mod ordering;
pub(crate) mod references;
mod storage;
mod tracking;
mod validation;

// Re-export the public API
pub use core::{add_node, put_node};
pub(crate) use tracking::track_reorder;
