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

//! In-memory LRU cache for workspace permissions.
//!
//! This module provides an efficient caching layer for workspace permissions to reduce
//! database lookups and improve authentication performance. The cache uses an LRU
//! (Least Recently Used) eviction policy and supports TTL-based expiration.
//!
//! # Architecture
//!
//! ```text
//! PermissionCache
//!   LRU Cache (Thread-Safe via RwLock)
//!     CacheKey(session_id, workspace_id) -> CachedPermissions
//!     - Max capacity: configurable
//!     - TTL: configurable (e.g., 5 minutes)
//!
//!   Invalidation:
//!     - By session (on logout)
//!     - By workspace (on permission change)
//!     - By TTL expiration
//! ```

mod permission_cache;
mod types;

#[cfg(test)]
mod tests;

pub use permission_cache::PermissionCache;
pub use types::{CacheKey, CachedPermissions};
