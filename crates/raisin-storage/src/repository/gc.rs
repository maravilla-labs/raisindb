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

//! Garbage collection storage trait

use raisin_error::Result;

use super::GarbageCollectionStats;

/// Garbage collection for revision and snapshot cleanup.
///
/// Implements mark-and-sweep garbage collection to remove unreferenced
/// revisions and their associated node snapshots.
pub trait GarbageCollectionRepository: Send + Sync {
    /// Run garbage collection for a repository
    ///
    /// This performs a mark-and-sweep garbage collection:
    /// 1. Mark: Starting from branch HEADs and tags, traverse parent chains
    ///    to mark all reachable revisions
    /// 2. Sweep: Delete all revisions and snapshots that weren't marked
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `dry_run` - If true, only report what would be deleted without deleting
    ///
    /// # Returns
    /// Statistics about the garbage collection run
    ///
    /// # Safety
    /// This operation should only be run when no other operations are modifying
    /// the repository to avoid race conditions.
    fn garbage_collect(
        &self,
        tenant_id: &str,
        repo_id: &str,
        dry_run: bool,
    ) -> impl std::future::Future<Output = Result<GarbageCollectionStats>> + Send;

    /// List unreferenced revisions without deleting them
    ///
    /// This is a lightweight version of garbage_collect that only identifies
    /// unreferenced revisions without performing deletion.
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    ///
    /// # Returns
    /// List of revision numbers that are unreferenced
    fn list_unreferenced_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<u64>>> + Send;
}
