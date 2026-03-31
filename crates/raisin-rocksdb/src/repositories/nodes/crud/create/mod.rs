//! Node creation operations
//!
//! This module contains all node creation operations:
//! - add_impl: Optimized creation for brand new nodes (skips existence checks)
//! - create_deep_node_impl: Deep node creation with automatic parent directory creation

mod add;
mod deep_create;
