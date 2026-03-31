//! Node operations module
//!
//! This module organizes node CRUD operations into focused submodules:
//! - `create`: Node creation operations (put_node, add_node)
//! - `read`: Node read operations (get_node, get_node_by_path)
//! - `delete`: Node deletion operations (delete_node, delete_path_index)
//! - `copy`: Node copy operations (copy_node_tree)
//! - `move_tree`: Node move operations (move_node_tree)
//! - `list`: Node list operations (list_children, scan_nodes)
//! - `deep`: Deep node operations (add_deep_node, upsert_deep_node)

pub(super) mod copy;
pub(super) mod create;
pub(super) mod deep;
pub(super) mod delete;
pub(super) mod list;
pub(super) mod move_tree;
pub(super) mod read;
pub(super) mod reorder;
pub(super) mod upsert;

// Re-export all public functions for use by parent module
pub(super) use copy::copy_node_tree;
pub(super) use create::{add_node, put_node};
pub(super) use deep::{add_deep_node, upsert_deep_node};
pub(super) use delete::{delete_node, delete_path_index};
pub(super) use list::{list_children, scan_nodes};
pub(super) use move_tree::move_node_tree;
pub(super) use read::{get_node, get_node_by_path};
pub(super) use reorder::{reorder_child_after, reorder_child_before};
pub(super) use upsert::upsert_node;
