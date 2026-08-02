// TODO(v0.2): Functions in this module are called dynamically via OperationApplicator dispatch
#![allow(dead_code)]

//! Operation application layer for replication
//!
//! This module handles applying operations received from peer nodes to the local database.
//! It implements a last-write-wins (LWW) conflict resolution strategy.
//!
//! ## Module Organization
//!
//! This module is organized by operation domain:
//! - `applicator` - Main OperationApplicator struct and operation dispatcher
//! - `node_operations` - Node CRUD operations (create, delete, update, move, etc.)
//! - `tenant_operations` - Tenant, deployment, and repository operations
//! - `user_operations` - User and workspace operations
//! - `tag_operations` - Tag creation and deletion operations
//! - `relation_operations` - Relation and archetype operations
//! - `schema_operations` - NodeType, Archetype, and ElementType operations
//!
//! ## Helper Modules
//!
//! - `conflict_resolution` - LWW conflict resolution helpers
//! - `db_helpers` - Database write/delete utilities
//! - `index_writers` - Index writing helpers for nodes

mod conflict_resolution;
mod db_helpers;
mod index_writers;
mod schema_operations;

// Domain-specific operation modules
mod identity_operations;
mod node_operations;
mod oauth_operations;
mod relation_operations;
mod tag_operations;
mod tenant_operations;
mod user_operations;

// Re-export the main applicator
mod applicator;
pub use applicator::OperationApplicator;
