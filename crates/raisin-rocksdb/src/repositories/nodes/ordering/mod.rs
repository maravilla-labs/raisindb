//! Child ordering operations using fractional indexing
//!
//! This module provides child ordering functionality organized into logical components:
//!
//! ## Module Organization
//!
//! - `queries` - Label and child ID query operations
//!   - Get order labels for specific children
//!   - Find adjacent labels for insert-between operations
//!   - Get ordered child ID lists
//!   - Find children by name
//!
//! - `reorder` - Shared reorder implementation
//!   - Core atomic write operations
//!   - Revision management
//!   - Metadata cache updates
//!
//! - `operations` - Public ordering operations
//!   - `reorder_child_impl` - Move child to numeric position
//!   - `move_child_before_impl` - Move child before another
//!   - `move_child_after_impl` - Move child after another
//!
//! ## Design Principles
//!
//! - **DRY**: Shared reorder logic eliminates ~600 lines of duplication
//! - **Efficiency**: O(1) label queries, no full node object loading
//! - **Atomicity**: All operations use WriteBatch for ACID guarantees
//! - **MVCC**: Full revision isolation with tombstone-based history
//! - **Locking**: Per-parent locks prevent concurrent modification races
//!
//! ## Fractional Indexing
//!
//! This module uses base-36 fractional indexing to maintain child order efficiently.
//! Labels are lexicographically sorted strings that allow insertion between any two
//! positions without rebalancing the entire list.

mod operations;
mod queries;
mod reorder;

// Re-export nothing - all functions are pub(super) and accessed via NodeRepositoryImpl
