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

//! Translation storage repository traits.
//!
//! Provides storage operations for managing multi-language translations
//! with revision awareness and block-level tracking.

use async_trait::async_trait;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{
    JsonPointer, LocaleCode, LocaleOverlay, TranslationHashRecord, TranslationMeta,
};

/// Translation storage repository trait.
///
/// Provides CRUD operations for storing and retrieving translations
/// with full revision awareness and block-level granularity.
///
/// # Key Concepts
///
/// - **Node Translations**: Per-locale overlays stored separately from base nodes
/// - **Block Translations**: UUID-based translations for Composite items
/// - **Revision Awareness**: Each translation change creates a new revision
/// - **Fallback Chains**: Configured at repository level, resolved at service level
///
/// # Storage Keys
///
/// - Translation data: `{tenant}\0{repo}\0{branch}\0{ws}\0translations\0{node_id}\0{locale}\0{~revision}`
/// - Block translations: `{tenant}\0{repo}\0{branch}\0{ws}\0block_trans\0{node_id}\0{block_uuid}\0{locale}\0{~revision}`
/// - Translation index: `{tenant}\0{repo}\0translation_index\0{locale}\0{~revision}\0{node_id}`
#[async_trait]
pub trait TranslationRepository: Send + Sync {
    /// Get the translation overlay for a node at a specific revision.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `locale` - Locale code
    /// * `revision` - Revision (HLC) for time-travel queries
    ///
    /// # Returns
    ///
    /// Returns the LocaleOverlay if found at or before the specified revision,
    /// or None if no translation exists.
    async fn get_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
        revision: &HLC,
    ) -> Result<Option<LocaleOverlay>>;

    /// Store a translation overlay for a node.
    ///
    /// Creates a new revision entry for the translation.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `locale` - Locale code
    /// * `overlay` - The translation overlay to store
    /// * `meta` - Translation metadata (revision, actor, message)
    async fn store_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
        overlay: &LocaleOverlay,
        meta: &TranslationMeta,
    ) -> Result<()>;

    /// Get a block-level translation by block UUID.
    ///
    /// Used for translating individual blocks within Composites,
    /// tracked by stable UUID rather than array position.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier (containing the block)
    /// * `block_uuid` - Block UUID
    /// * `locale` - Locale code
    /// * `revision` - Revision (HLC)
    ///
    /// # Returns
    ///
    /// Returns the translated block overlay if found, or None.
    async fn get_block_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        block_uuid: &str,
        locale: &LocaleCode,
        revision: &HLC,
    ) -> Result<Option<LocaleOverlay>>;

    /// Store a block-level translation.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier (containing the block)
    /// * `block_uuid` - Block UUID
    /// * `locale` - Locale code
    /// * `overlay` - The translation overlay for this block
    /// * `meta` - Translation metadata
    async fn store_block_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        block_uuid: &str,
        locale: &LocaleCode,
        overlay: &LocaleOverlay,
        meta: &TranslationMeta,
    ) -> Result<()>;

    /// List all translations available for a node at a given revision.
    ///
    /// Returns the set of locales that have translations for this node.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `revision` - Revision (HLC)
    ///
    /// # Returns
    ///
    /// Vector of locale codes that have translations at or before this revision.
    async fn list_translations_for_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        revision: &HLC,
    ) -> Result<Vec<LocaleCode>>;

    /// List all nodes that have translations in a specific locale.
    ///
    /// Used for reverse lookups: "which nodes are translated to French?"
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `locale` - Locale code to query
    /// * `revision` - Revision (HLC)
    ///
    /// # Returns
    ///
    /// Vector of node IDs that have translations in this locale.
    async fn list_nodes_with_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        locale: &LocaleCode,
        revision: &HLC,
    ) -> Result<Vec<String>>;

    /// Mark blocks as orphaned (deleted from base node).
    ///
    /// When blocks are removed from the base node, we mark their translations
    /// as orphaned rather than deleting them. This allows for audit trails
    /// and potential recovery if the block is re-added.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `block_uuids` - List of block UUIDs to mark as orphaned
    /// * `revision` - Revision (HLC) of the deletion
    async fn mark_blocks_orphaned(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        block_uuids: &[String],
        revision: &HLC,
    ) -> Result<()>;

    /// Get the latest translation metadata for a node/locale.
    ///
    /// Returns metadata about the most recent translation update.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `locale` - Locale code
    ///
    /// # Returns
    ///
    /// The most recent TranslationMeta, or None if no translation exists.
    async fn get_translation_meta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
    ) -> Result<Option<TranslationMeta>>;

    /// Batch fetch translations for multiple nodes in a single locale.
    ///
    /// This method is optimized for bulk translation resolution when building trees.
    /// Instead of making N individual get_translation() calls, this fetches all
    /// translations in a single operation using RocksDB prefix scans or multi-get.
    ///
    /// # Performance
    ///
    /// - O(k) where k = number of nodes with translations
    /// - 10-100x faster than individual get_translation() calls
    /// - Uses RocksDB MultiGet or prefix iteration
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_ids` - List of node IDs to fetch translations for
    /// * `locale` - Locale code to fetch
    /// * `revision` - Revision (HLC) for snapshot isolation
    ///
    /// # Returns
    ///
    /// HashMap where key is node_id and value is the LocaleOverlay.
    /// Only nodes that have translations in this locale are included.
    /// If a node has no translation, it won't be in the result map.
    ///
    /// # Example
    ///
    /// ```text
    /// node_ids=["article-1", "article-2", "article-3"], locale="fr"
    /// Returns:
    /// {
    ///   "article-1": LocaleOverlay::Properties { ... },
    ///   "article-3": LocaleOverlay::Hidden
    /// }
    /// // Note: article-2 has no French translation, so not in results
    /// ```
    async fn get_translations_batch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_ids: &[String],
        locale: &LocaleCode,
        revision: &HLC,
    ) -> Result<std::collections::HashMap<String, LocaleOverlay>>;

    // ========================================================================
    // Translation Hash Record Methods (for staleness detection)
    // ========================================================================

    /// Store a hash record for a translation field.
    ///
    /// Records the hash of the original content at the time of translation,
    /// allowing later detection of whether the original has changed.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `locale` - Locale code
    /// * `pointer` - JSON Pointer to the translated field
    /// * `record` - The hash record to store
    async fn store_hash_record(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
        pointer: &JsonPointer,
        record: &TranslationHashRecord,
    ) -> Result<()>;

    /// Store multiple hash records in a single operation.
    ///
    /// More efficient than calling `store_hash_record` multiple times.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `locale` - Locale code
    /// * `records` - Map of JSON Pointers to hash records
    async fn store_hash_records_batch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
        records: &std::collections::HashMap<JsonPointer, TranslationHashRecord>,
    ) -> Result<()>;

    /// Get all hash records for a node/locale combination.
    ///
    /// Retrieves the hash records for all translated fields, allowing
    /// staleness detection by comparing against current original hashes.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `locale` - Locale code
    ///
    /// # Returns
    ///
    /// HashMap of JSON Pointers to hash records.
    async fn get_hash_records(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
    ) -> Result<std::collections::HashMap<JsonPointer, TranslationHashRecord>>;

    /// Delete hash records for a node/locale (when translation is deleted).
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace` - Workspace identifier
    /// * `node_id` - Node identifier
    /// * `locale` - Locale code
    async fn delete_hash_records(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
    ) -> Result<()>;
}
