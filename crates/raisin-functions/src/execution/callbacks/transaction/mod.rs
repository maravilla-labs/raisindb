// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Transaction operation callbacks for function execution.
//!
//! These callbacks implement the `raisin.nodes.beginTransaction()` API available to JavaScript functions.
//! Transactions are implemented using SQL BEGIN/COMMIT to ensure consistent behavior with
//! the SQL execution engine. All operations within a transaction generate SQL statements
//! that are executed via QueryEngine.
//!
//! ## Submodules
//!
//! - [`store`] - `TransactionStore` for managing active QueryEngine instances
//! - [`helpers`] - SQL parameter substitution, JSON/PropertyValue conversion, node parsing
//! - [`lifecycle`] - Begin, commit, rollback, set_actor, set_message callbacks
//! - [`read_ops`] - Get, get_by_path, list_children (read-only queries)
//! - [`write_ops`] - Create, add, put, upsert, update, delete, move, and property callbacks

mod helpers;
pub mod store;

pub mod lifecycle;
pub mod read_ops;
pub mod write_ops;

// Re-export TransactionStore at the module root for backwards compatibility.
pub use store::TransactionStore;

// Re-export lifecycle callbacks.
pub use lifecycle::{
    create_tx_begin, create_tx_commit, create_tx_rollback, create_tx_set_actor,
    create_tx_set_message,
};

// Re-export read operation callbacks.
pub use read_ops::{create_tx_get, create_tx_get_by_path, create_tx_list_children};

// Re-export write operation callbacks.
pub use write_ops::{
    create_tx_add, create_tx_create, create_tx_create_deep, create_tx_delete,
    create_tx_delete_by_id, create_tx_move, create_tx_put, create_tx_update,
    create_tx_update_property, create_tx_upsert, create_tx_upsert_deep,
};
