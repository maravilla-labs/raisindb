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

mod child_placement;
mod child_probe;
mod key_parse;
mod label_lookup;
mod labels;
mod operations;
mod paged;
mod paged_desc;
mod put_entry;
mod queries;
mod rebalance;
mod reorder;
mod tree_order;

pub(crate) use child_placement::{child_is_under, node_path_at};
pub(crate) use key_parse::parse_ordered_child_key;
pub(crate) use label_lookup::{
    current_order_label, last_live_order_label, live_entry_under_label, CurrentLabel,
};
pub(in crate::repositories::nodes) use labels::format_order_label;
pub(crate) use labels::{mint_append_label, sorts_after};
pub(in crate::repositories::nodes) use paged::{OrderedChildEntry, OrderedScanStart};
pub(crate) use put_entry::put_ordered_child;
pub(crate) use queries::{parent_index_id, stored_order_label, stored_order_label_at};
pub(in crate::repositories::nodes) use tree_order::{join_tree_order, split_tree_order};

// Re-export nothing - all functions are pub(super) and accessed via NodeRepositoryImpl
