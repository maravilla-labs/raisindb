//! Node service for managing nodes within workspaces.
//!
//! Provides CRUD operations, validation, tree management, and publication workflows
//! for nodes. Nodes are validated against NodeType schemas and organized in a
//! hierarchical tree structure within workspaces.

// Core struct definition and constructors
mod core;
pub use self::core::NodeService;

// RLS (Row-Level Security) helpers
mod rls_helpers;

// Revision / time-travel helpers
mod revision;

// Workspace delta overlay
mod workspace_delta;

// UpdateBuilder fluent API
mod update_builder;
pub use update_builder::UpdateBuilder;

// CRUD operations (read, create, update, delete, list, internal helpers)
mod crud;

// Feature submodules
mod branch_tag_operations;
mod copy_publish;
mod node_creation_helpers;
mod property_operations;
mod relationship_operations;
mod transactional;
mod tree_operations;
mod versioning;

#[cfg(test)]
mod tests_publish_delete;

// Re-export relationship types
pub use relationship_operations::{IncomingRelation, NodeRelationships};
