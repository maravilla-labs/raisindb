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

//! Shared parsing and assembly for `raisin:Integration` connectors.
//!
//! # Why this lives in `raisin-models`
//!
//! The sync engine (`raisin-rocksdb`) and the HTTP connection-test handler
//! (`raisin-transport-http`) both have to turn a connector node plus a chosen
//! connection into (a) the config an adapter sees and (b) the credential it
//! authenticates with. Those two assemblies used to be *duplicated* — one copy
//! in `virtual_mount_sync::adapter`, another in `test_connection::support` —
//! which is this codebase's most reliable bug factory: the pair drifts, and the
//! symptom is "it syncs but the test says otherwise" (or worse, the reverse).
//! Both crates already depend on `raisin-models`, so the shared shape belongs
//! here.
//!
//! This module is deliberately **pure**: no I/O, no crypto. Callers decrypt
//! blobs themselves and hand in plaintext maps, which keeps `raisin-crypto` out
//! of the model layer and makes every rule below unit-testable.

mod account;
mod capabilities;
mod config;
mod credential;

pub use account::{AccountSelection, AccountSelectionError, AuthKind, ConnectedAccount};
pub use capabilities::Capabilities;
pub use config::{merge_config, secret_field_names, IntegrationConfig};
pub use credential::build_credential;

#[cfg(test)]
mod tests;
