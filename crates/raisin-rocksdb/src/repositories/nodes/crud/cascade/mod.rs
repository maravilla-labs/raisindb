//! Cascade delete operations for nodes
//!
//! This module provides recursive deletion of node trees, ensuring no orphaned
//! nodes are left in the database. It has been refactored from a single large
//! file into smaller, focused modules for better maintainability:
//!
//! - `tombstone`: Shared tombstone writing logic (eliminates duplication)
//! - `single`: Single node deletion operations
//! - `tree`: Tree/cascade deletion operations
//!
//! # Key Design Principles
//!
//! 1. **DRY (Don't Repeat Yourself)**: All tombstone writing logic is centralized
//!    in the `tombstone` module's `add_node_tombstones_to_batch` function. This
//!    eliminates the previous duplication across multiple deletion functions.
//!
//! 2. **Single Responsibility**: Each module focuses on one aspect:
//!    - `tombstone.rs`: How to write tombstones
//!    - `single.rs`: Single node deletion
//!    - `tree.rs`: Tree deletion operations
//!
//! 3. **Performance**: All operations use optimized WriteBatch operations and
//!    a single revision for entire tree deletions.
//!
//! # Usage
//!
//! Public API (accessible from NodeRepository trait):
//! - `delete_with_cascade`: Delete a node and all descendants
//! - `delete_without_cascade`: Delete a node only if it has no children
//!
//! Internal helpers (pub(in super::super)):
//! - `delete_node_with_revision`: Delete single node with specific revision
//! - `delete_descendants_with_revision`: Delete all descendants with revision
//! - `delete_tree_with_single_batch`: Optimized whole-tree deletion
//! - `add_node_tombstones_to_batch`: Core tombstone writing logic

mod single;
mod tombstone;
mod tree;

#[cfg(test)]
mod tests {
    // NOTE: Full integration tests will be in the integration test suite
    // These tests require a full RocksDB setup with test fixtures

    #[test]
    fn test_cascade_module_exists() {
        // Placeholder test to ensure module compiles
        assert!(true);
    }

    // TODO: Add integration tests:
    // - test_delete_with_cascade_single_level (parent + 3 children)
    // - test_delete_with_cascade_multi_level (parent + children + grandchildren)
    // - test_delete_with_cascade_empty (node with no children)
    // - test_delete_without_cascade_fails_with_children
    // - test_delete_without_cascade_succeeds_without_children
    // - test_cascade_preserves_siblings (deleting one branch doesn't affect others)
    // - test_cascade_performance (1000 nodes deleted in <1s)
}
