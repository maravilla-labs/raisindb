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

//! Cached trigger registry for optimizing trigger matching
//!
//! This module provides a memory-cached registry of all triggers with inverted
//! indexes for fast lookup. It eliminates the need to query the database for
//! every node event by providing O(1) quick-reject capability.
//!
//! ## Architecture
//!
//! - **Immutable Snapshots**: Each snapshot is never modified after creation
//! - **Atomic Swaps**: Updates use arc-swap for lock-free reads
//! - **Inverted Indexes**: Pre-computed indexes by workspace, node_type, event_kind
//! - **Quick Reject**: Can instantly determine if an event can't possibly match

mod parsers;
mod registry;
pub(crate) mod snapshot;
mod types;

#[cfg(test)]
mod tests;

pub use registry::TriggerRegistry;
pub use types::{CachedTrigger, TriggerFilters};
