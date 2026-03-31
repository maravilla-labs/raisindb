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

//! Types for the permission cache.
//!
//! Contains the cache key and cached permissions types used by `PermissionCache`.

use std::time::{Duration, Instant};

/// Cache key for workspace permissions.
///
/// Combines session ID and workspace ID to uniquely identify a permission context.
/// Uses both fields because:
/// - Different sessions (users) have different permissions for the same workspace
/// - Same session has different permissions across different workspaces
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct CacheKey {
    /// Session identifier
    pub session_id: String,
    /// Workspace identifier
    pub workspace_id: String,
}

impl CacheKey {
    /// Create a new cache key.
    pub fn new(session_id: impl Into<String>, workspace_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            workspace_id: workspace_id.into(),
        }
    }
}

/// Cached workspace permissions.
///
/// Contains all permission information for a user in a workspace.
/// Includes metadata for cache invalidation and versioning.
#[derive(Clone, Debug)]
pub struct CachedPermissions {
    /// User's node ID in the graph database
    pub user_node_id: String,
    /// Roles assigned to the user in this workspace
    pub roles: Vec<String>,
    /// Groups the user belongs to in this workspace
    pub groups: Vec<String>,
    /// Whether the user is a workspace administrator
    pub is_workspace_admin: bool,
    /// When these permissions were resolved from the database
    pub resolved_at: Instant,
    /// Version number for permission changes (used for cache invalidation)
    pub permissions_version: u64,
}

impl CachedPermissions {
    /// Check if the cached permissions have expired based on TTL.
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.resolved_at.elapsed() > ttl
    }

    /// Create a new cached permissions instance.
    pub fn new(
        user_node_id: impl Into<String>,
        roles: Vec<String>,
        groups: Vec<String>,
        is_workspace_admin: bool,
        permissions_version: u64,
    ) -> Self {
        Self {
            user_node_id: user_node_id.into(),
            roles,
            groups,
            is_workspace_admin,
            resolved_at: Instant::now(),
            permissions_version,
        }
    }
}
