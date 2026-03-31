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

//! Write/mutating transaction operation callbacks.
//!
//! Covers create, add, put, upsert, create_deep, upsert_deep, update,
//! delete, delete_by_id, move, and update_property operations that
//! modify data inside an active SQL transaction.
//!
//! ## Submodules
//!
//! - [`simple_ops`] - Single-statement create, add, put, upsert
//! - [`deep_ops`] - Multi-statement create_deep, upsert_deep with auto parent creation
//! - [`mutation_ops`] - Update and update_property for modifying existing nodes
//! - [`delete_ops`] - Delete, delete_by_id, and move operations

mod deep_ops;
mod delete_ops;
mod mutation_ops;
mod simple_ops;

pub use deep_ops::{create_tx_create_deep, create_tx_upsert_deep};
pub use delete_ops::{create_tx_delete, create_tx_delete_by_id, create_tx_move};
pub use mutation_ops::{create_tx_update, create_tx_update_property};
pub use simple_ops::{create_tx_add, create_tx_create, create_tx_put, create_tx_upsert};
