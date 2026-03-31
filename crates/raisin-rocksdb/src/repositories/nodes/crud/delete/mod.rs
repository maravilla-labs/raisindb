//! Node deletion operations
//!
//! This module contains all delete operations for nodes:
//! - delete_impl: Soft delete using tombstones
//! - check_delete_safety: Verify referential integrity before deletion

mod safety_check;
mod tombstone;
