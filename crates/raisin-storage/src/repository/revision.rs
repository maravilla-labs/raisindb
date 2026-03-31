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

//! Revision and commit tracking storage trait

use raisin_error::Result;
use raisin_hlc::HLC;

use super::RevisionMeta;

/// Revision and commit tracking storage operations.
///
/// Provides operations for managing immutable revisions and commit history.
pub trait RevisionRepository: Send + Sync {
    /// Allocate a new revision number for a repository
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    ///
    /// # Returns
    /// New revision number
    /// Allocate a new HLC revision (lock-free, no parameters needed)
    ///
    /// This is a synchronous operation that ticks the internal HLC state.
    /// No tenant_id or repo_id needed - HLC is node-local and globally unique.
    fn allocate_revision(&self) -> HLC;

    /// Store revision metadata
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `meta` - Revision metadata
    fn store_revision_meta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        meta: RevisionMeta,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Get revision metadata
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `revision` - Revision number
    ///
    /// # Returns
    /// Revision metadata if it exists
    fn get_revision_meta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
    ) -> impl std::future::Future<Output = Result<Option<RevisionMeta>>> + Send;

    /// List revisions in a repository (newest first)
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `limit` - Maximum number of revisions to return
    /// * `offset` - Number of revisions to skip (for pagination)
    ///
    /// # Returns
    /// Vector of revision metadata
    fn list_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        limit: usize,
        offset: usize,
    ) -> impl std::future::Future<Output = Result<Vec<RevisionMeta>>> + Send;

    /// List nodes changed in a specific revision
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `revision` - Revision number
    ///
    /// # Returns
    /// Vector of node changes with operation types (added, modified, deleted)
    fn list_changed_nodes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
    ) -> impl std::future::Future<Output = Result<Vec<raisin_models::tree::NodeChange>>> + Send;

    /// Store reverse index entry (revision -> changed node)
    ///
    /// This enables efficient lookup of "which revisions touched this node?"
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `revision` - Revision number
    /// * `node_id` - Node identifier
    fn index_node_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Store reverse index entry (revision -> changed node type)
    ///
    /// Enables lookup of revisions that touched a specific NodeType.
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `revision` - Revision number
    /// * `node_type_name` - NodeType name
    fn index_node_type_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        node_type_name: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Store reverse index entry (revision -> changed archetype)
    ///
    /// Enables lookup of revisions that touched a specific Archetype.
    fn index_archetype_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        archetype_name: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Store reverse index entry (revision -> changed element type)
    ///
    /// Enables lookup of revisions that touched a specific ElementType.
    fn index_element_type_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        element_type_name: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Get revisions that changed a specific node
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `node_id` - Node identifier
    /// * `limit` - Maximum number of revisions to return
    ///
    /// # Returns
    /// Vector of revision numbers (newest first)
    fn get_node_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<HLC>>> + Send;

    /// Get revisions that changed a specific NodeType
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `node_type_name` - NodeType name
    /// * `limit` - Maximum number of revisions to return
    fn get_node_type_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_type_name: &str,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<HLC>>> + Send;

    /// Get revisions that changed a specific Archetype
    fn get_archetype_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        archetype_name: &str,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<HLC>>> + Send;

    /// Get revisions that changed a specific ElementType
    fn get_element_type_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        element_type_name: &str,
        limit: usize,
    ) -> impl std::future::Future<Output = Result<Vec<HLC>>> + Send;

    /// Store a node snapshot at a specific revision
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `node_id` - Node identifier
    /// * `revision` - Revision number
    /// * `node_json` - Serialized node data
    fn store_node_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        revision: &HLC,
        node_json: Vec<u8>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Get a node snapshot at a specific revision
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `node_id` - Node identifier
    /// * `revision` - Revision number
    ///
    /// # Returns
    /// Serialized node data if snapshot exists
    fn get_node_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        revision: &HLC,
    ) -> impl std::future::Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Get node snapshot at or before a specific revision (time-travel)
    ///
    /// This is the key method for revision-aware node retrieval.
    /// Returns the most recent snapshot of a node at or before the given revision.
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `node_id` - Node identifier
    /// * `revision` - Maximum revision number to consider
    ///
    /// # Returns
    /// Tuple of (actual_revision, node_json) if a snapshot exists
    fn get_node_snapshot_at_or_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        revision: &HLC,
    ) -> impl std::future::Future<Output = Result<Option<(HLC, Vec<u8>)>>> + Send;

    /// Store a translation snapshot at a specific revision
    ///
    /// Enables time-travel queries for translations and rollback capabilities.
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `node_id` - Node identifier
    /// * `locale` - Locale code (e.g., "fr", "de-CH")
    /// * `revision` - Revision number
    /// * `overlay_json` - Serialized LocaleOverlay data
    fn store_translation_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        locale: &str,
        revision: &HLC,
        overlay_json: Vec<u8>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Get a translation snapshot at a specific revision
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `node_id` - Node identifier
    /// * `locale` - Locale code
    /// * `revision` - Revision number
    ///
    /// # Returns
    /// Serialized LocaleOverlay data if snapshot exists
    fn get_translation_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        locale: &str,
        revision: &HLC,
    ) -> impl std::future::Future<Output = Result<Option<Vec<u8>>>> + Send;

    /// Get translation snapshot at or before a specific revision (time-travel)
    ///
    /// Returns the most recent translation snapshot at or before the given revision.
    /// This is used for translation rollback and historical queries.
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `node_id` - Node identifier
    /// * `locale` - Locale code
    /// * `revision` - Maximum revision number to consider
    ///
    /// # Returns
    /// Tuple of (actual_revision, overlay_json) if a snapshot exists
    fn get_translation_snapshot_at_or_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        locale: &str,
        revision: &HLC,
    ) -> impl std::future::Future<Output = Result<Option<(HLC, Vec<u8>)>>> + Send;
}
