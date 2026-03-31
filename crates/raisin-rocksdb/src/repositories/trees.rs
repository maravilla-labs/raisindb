//! Tree repository implementation for content-addressed storage

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::tree::TreeEntry;
use raisin_storage::scope::RepoScope;
use raisin_storage::TreeRepository;
use rocksdb::DB;
use std::sync::Arc;

#[derive(Clone)]
pub struct TreeRepositoryImpl {
    db: Arc<DB>,
}

impl TreeRepositoryImpl {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }
}

impl TreeRepository for TreeRepositoryImpl {
    async fn build_leaf(&self, scope: RepoScope<'_>, entries: &[TreeEntry]) -> Result<[u8; 32]> {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        // Sort entries for consistent hashing
        let mut sorted_entries = entries.to_vec();
        sorted_entries.sort_by(|a, b| a.entry_key.cmp(&b.entry_key));

        // Serialize and hash
        let serialized = rmp_serde::to_vec(&sorted_entries)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let tree_id = blake3::hash(&serialized);

        // Store tree
        let key = keys::tree_key(tenant_id, repo_id, tree_id.as_bytes());
        let cf = cf_handle(&self.db, cf::TREES)?;

        self.db
            .put_cf(cf, key, serialized)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(*tree_id.as_bytes())
    }

    async fn iter_tree(
        &self,
        scope: RepoScope<'_>,
        tree_id: &[u8; 32],
        start_after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TreeEntry>> {
        let tenant_id = scope.tenant_id;
        let repo_id = scope.repo_id;
        let key = keys::tree_key(tenant_id, repo_id, tree_id);
        let cf = cf_handle(&self.db, cf::TREES)?;

        let value = self
            .db
            .get_cf(cf, key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            .ok_or_else(|| raisin_error::Error::NotFound("Tree not found".to_string()))?;

        let entries: Vec<TreeEntry> = rmp_serde::from_slice(&value)
            .map_err(|e| raisin_error::Error::storage(format!("Deserialization error: {}", e)))?;

        let filtered: Vec<TreeEntry> = if let Some(after) = start_after {
            entries
                .into_iter()
                .skip_while(|e| e.entry_key.as_str() <= after)
                .take(limit)
                .collect()
        } else {
            entries.into_iter().take(limit).collect()
        };

        Ok(filtered)
    }

    async fn get_tree_entry(
        &self,
        scope: RepoScope<'_>,
        tree_id: &[u8; 32],
        entry_key: &str,
    ) -> Result<Option<TreeEntry>> {
        let entries = self.iter_tree(scope, tree_id, None, usize::MAX).await?;

        Ok(entries.into_iter().find(|e| e.entry_key == entry_key))
    }

    async fn get_root_tree_id(
        &self,
        scope: RepoScope<'_>,
        revision: &HLC,
    ) -> Result<Option<[u8; 32]>> {
        // This would normally be stored in revision metadata
        // For now, return None as a stub
        let _scope = scope;
        Ok(None)
    }
}
