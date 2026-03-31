//! RocksDB implementation of the TranslationRepository trait.
//!
//! Provides storage for node translations with revision awareness,
//! block-level tracking, and reverse indexes.
//!
//! # Module Organization
//!
//! - `keys`: Key encoding functions for all translation storage
//! - `serialization`: Serialization/deserialization helpers
//! - `nodes`: Node-level translation CRUD operations
//! - `blocks`: Block-level translation CRUD operations
//! - `queries`: Translation query operations
//! - `metadata`: Translation metadata operations
//! - `revision`: RevisionMeta creation and snapshot storage
//! - `replication`: Operation capture for replication

mod blocks;
mod hash_store;
mod keys;
mod metadata;
mod nodes;
mod queries;
mod replication;
mod revision;
mod serialization;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::translations::{
    JsonPointer, LocaleCode, LocaleOverlay, TranslationHashRecord, TranslationMeta,
};
use raisin_storage::TranslationRepository;
use rocksdb::DB;
use std::sync::Arc;

/// RocksDB-backed translation repository implementation.
///
/// Stores translations in three column families:
/// - `TRANSLATION_DATA`: Node-level translations
/// - `BLOCK_TRANSLATIONS`: Block-level translations (by UUID)
/// - `TRANSLATION_INDEX`: Reverse index (locale -> nodes)
#[derive(Clone)]
pub struct RocksDBTranslationRepository {
    db: Arc<DB>,
    operation_capture: Option<Arc<crate::OperationCapture>>,
}

impl RocksDBTranslationRepository {
    /// Create a new RocksDB translation repository
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            operation_capture: None,
        }
    }

    /// Create a new RocksDB translation repository with operation capture
    pub fn new_with_capture(db: Arc<DB>, operation_capture: Arc<crate::OperationCapture>) -> Self {
        Self {
            db,
            operation_capture: Some(operation_capture),
        }
    }
}

#[async_trait]
impl TranslationRepository for RocksDBTranslationRepository {
    async fn get_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
        revision: &HLC,
    ) -> Result<Option<LocaleOverlay>> {
        nodes::get_translation(
            &self.db, tenant_id, repo_id, branch, workspace, node_id, locale, revision,
        )
        .await
    }

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
    ) -> Result<()> {
        nodes::store_translation(
            &self.db,
            self.operation_capture.as_ref(),
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            locale,
            overlay,
            meta,
        )
        .await
    }

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
    ) -> Result<Option<LocaleOverlay>> {
        blocks::get_block_translation(
            &self.db, tenant_id, repo_id, branch, workspace, node_id, block_uuid, locale, revision,
        )
        .await
    }

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
    ) -> Result<()> {
        blocks::store_block_translation(
            &self.db,
            self.operation_capture.as_ref(),
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            block_uuid,
            locale,
            overlay,
            meta,
        )
        .await
    }

    async fn list_translations_for_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        revision: &HLC,
    ) -> Result<Vec<LocaleCode>> {
        nodes::list_translations_for_node(
            &self.db, tenant_id, repo_id, branch, workspace, node_id, revision,
        )
        .await
    }

    async fn list_nodes_with_translation(
        &self,
        tenant_id: &str,
        repo_id: &str,
        locale: &LocaleCode,
        revision: &HLC,
    ) -> Result<Vec<String>> {
        queries::list_nodes_with_translation(&self.db, tenant_id, repo_id, locale, revision).await
    }

    async fn mark_blocks_orphaned(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        block_uuids: &[String],
        revision: &HLC,
    ) -> Result<()> {
        blocks::mark_blocks_orphaned(
            &self.db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            block_uuids,
            revision,
        )
        .await
    }

    async fn get_translation_meta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
    ) -> Result<Option<TranslationMeta>> {
        metadata::get_translation_meta(
            &self.db, tenant_id, repo_id, branch, workspace, node_id, locale,
        )
        .await
    }

    async fn get_translations_batch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_ids: &[String],
        locale: &LocaleCode,
        revision: &HLC,
    ) -> Result<std::collections::HashMap<String, LocaleOverlay>> {
        queries::get_translations_batch(
            &self.db, tenant_id, repo_id, branch, workspace, node_ids, locale, revision,
        )
        .await
    }

    // ========================================================================
    // Translation Hash Record Methods (for staleness detection)
    // ========================================================================

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
    ) -> Result<()> {
        hash_store::store_hash_record(
            &self.db, tenant_id, repo_id, branch, workspace, node_id, locale, pointer, record,
        )
        .await
    }

    async fn store_hash_records_batch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
        records: &std::collections::HashMap<JsonPointer, TranslationHashRecord>,
    ) -> Result<()> {
        hash_store::store_hash_records_batch(
            &self.db, tenant_id, repo_id, branch, workspace, node_id, locale, records,
        )
        .await
    }

    async fn get_hash_records(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
    ) -> Result<std::collections::HashMap<JsonPointer, TranslationHashRecord>> {
        hash_store::get_hash_records(
            &self.db, tenant_id, repo_id, branch, workspace, node_id, locale,
        )
        .await
    }

    async fn delete_hash_records(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
    ) -> Result<()> {
        hash_store::delete_hash_records(
            &self.db, tenant_id, repo_id, branch, workspace, node_id, locale,
        )
        .await
    }
}
