//! Basic CRUD operations for NodeService
//!
//! This module contains the core read/write operations for nodes:
//! - get, get_by_path (read.rs)
//! - create, put (deprecated) (create.rs)
//! - update_node, upsert (update.rs)
//! - delete, delete_by_path (delete.rs)
//! - list_by_type, list_by_parent, list_root, list_all, has_children (list.rs)
//! - put_without_versioning (internal) (internal.rs)

mod create;
mod delete;
mod internal;
mod list;
mod read;
mod update;

// All methods are implemented directly on NodeService via impl blocks in each submodule.
// No re-exports needed - the methods are part of the NodeService type.
