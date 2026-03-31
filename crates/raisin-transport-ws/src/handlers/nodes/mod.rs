// SPDX-License-Identifier: BSL-1.1

//! Node operation handlers.
//!
//! Submodules are organized by operation category:
//!
//! - [`crud`] -- create, update, delete, get
//! - [`query`] -- query, query_by_path, query_by_property
//! - [`sql_query`] -- SQL query execution with INVOKE support
//! - [`operations`] -- move, rename, copy, reorder
//! - [`tree`] -- list_children, get_tree, get_tree_flat
//! - [`properties`] -- property get/update by dot-path
//! - [`relations`] -- add, remove, get relationships

mod crud;
mod helpers;
mod operations;
mod properties;
mod query;
mod relations;
mod sql_query;
mod tree;

// Re-export all public handler functions so callers can use the same paths.
pub use crud::{handle_node_create, handle_node_delete, handle_node_get, handle_node_update};
pub use operations::{
    handle_node_copy, handle_node_copy_tree, handle_node_move, handle_node_move_child_after,
    handle_node_move_child_before, handle_node_rename, handle_node_reorder,
};
pub use properties::{handle_property_get, handle_property_update};
pub use query::{handle_node_query, handle_node_query_by_path, handle_node_query_by_property};
pub use relations::{handle_relation_add, handle_relation_remove, handle_relations_get};
pub use sql_query::handle_sql_query;
pub use tree::{handle_node_get_tree, handle_node_get_tree_flat, handle_node_list_children};
