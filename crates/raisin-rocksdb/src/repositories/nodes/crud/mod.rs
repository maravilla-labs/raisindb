//! Core CRUD operations for nodes
//!
//! This module is split into smaller, focused sub-modules for better maintainability:
//!
//! - `create`: Node creation operations (add_impl, create_deep_node_impl)
//! - `update`: Node update operations (update_impl)
//! - `read`: Node retrieval operations (get_impl, get_at_revision_impl, list_all_impl, count_all_impl)
//! - `delete`: Node deletion operations (delete_impl, check_delete_safety)
//! - `cascade`: Cascade deletion operations (delete_with_cascade, delete_without_cascade)
//! - `batch`: Batch operation helpers (add_node_to_batch, add_ordered_children_to_batch)
//! - `indexing`: Shared indexing logic (DRY - used by create and update)
//! - `helpers`: Common CRUD helper functions (revision lookups, relation queries)
//!
//! All functions are implemented as methods on NodeRepositoryImpl and are accessible
//! via their respective impl blocks in each sub-module.

mod batch;
mod cascade;
mod create;
mod delete;
mod helpers;
mod indexing;
mod read;
mod update;
