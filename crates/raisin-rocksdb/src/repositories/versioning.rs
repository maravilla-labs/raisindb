//! Versioning repository implementation

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::{Node, NodeVersion};
use raisin_storage::VersioningRepository;
use rocksdb::DB;
use std::sync::Arc;

#[derive(Clone)]
pub struct VersioningRepositoryImpl {
    db: Arc<DB>,
}

impl VersioningRepositoryImpl {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    fn get_next_version(&self, tenant_id: &str, repo_id: &str, node_id: &str) -> Result<i32> {
        let versions = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.list_versions(node_id))
        })?;

        Ok(versions.len() as i32 + 1)
    }
}

impl VersioningRepository for VersioningRepositoryImpl {
    async fn create_version_with_note(&self, node: &Node, note: Option<String>) -> Result<i32> {
        let tenant_id = "default"; // TODO: Get from context
        let repo_id = "default"; // TODO: Get from context
        let version_num = self.get_next_version(tenant_id, repo_id, &node.id)?;

        let version = NodeVersion {
            id: nanoid::nanoid!(),
            node_id: node.id.clone(),
            version: version_num,
            node_data: node.clone(),
            created_at: Some(chrono::Utc::now()),
            updated_at: None,
            note,
        };

        let key = keys::version_key(tenant_id, repo_id, &node.id, version_num);
        let value = rmp_serde::to_vec(&version)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::VERSIONS)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(version_num)
    }

    async fn list_versions(&self, node_id: &str) -> Result<Vec<NodeVersion>> {
        let tenant_id = "default"; // TODO: Get from context
        let repo_id = "default"; // TODO: Get from context

        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("versions")
            .push(node_id)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::VERSIONS)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut versions = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let version: NodeVersion = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;
            versions.push(version);
        }

        versions.sort_by_key(|v| v.version);

        Ok(versions)
    }

    async fn get_version(&self, node_id: &str, version: i32) -> Result<Option<NodeVersion>> {
        let tenant_id = "default"; // TODO: Get from context
        let repo_id = "default"; // TODO: Get from context

        let key = keys::version_key(tenant_id, repo_id, node_id, version);
        let cf = cf_handle(&self.db, cf::VERSIONS)?;

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                let version: NodeVersion = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                Ok(Some(version))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }

    async fn delete_all_versions(&self, node_id: &str) -> Result<usize> {
        let tenant_id = "default"; // TODO: Get from context
        let repo_id = "default"; // TODO: Get from context

        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("versions")
            .push(node_id)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::VERSIONS)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut count = 0;

        for item in iter {
            let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            self.db
                .delete_cf(cf, key)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            count += 1;
        }

        Ok(count)
    }

    async fn delete_version(&self, node_id: &str, version: i32) -> Result<bool> {
        let tenant_id = "default"; // TODO: Get from context
        let repo_id = "default"; // TODO: Get from context

        let key = keys::version_key(tenant_id, repo_id, node_id, version);
        let cf = cf_handle(&self.db, cf::VERSIONS)?;

        let exists = self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            .is_some();

        if exists {
            self.db
                .delete_cf(cf, key)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn delete_old_versions(&self, node_id: &str, keep_count: usize) -> Result<usize> {
        let versions = self.list_versions(node_id).await?;

        if versions.len() <= keep_count {
            return Ok(0);
        }

        let to_delete = versions.len() - keep_count;
        let mut deleted = 0;

        for version in versions.iter().take(to_delete) {
            if self.delete_version(node_id, version.version).await? {
                deleted += 1;
            }
        }

        Ok(deleted)
    }

    async fn update_version_note(
        &self,
        node_id: &str,
        version: i32,
        note: Option<String>,
    ) -> Result<()> {
        if let Some(mut v) = self.get_version(node_id, version).await? {
            v.note = note;

            let tenant_id = "default"; // TODO: Get from context
            let repo_id = "default"; // TODO: Get from context

            let key = keys::version_key(tenant_id, repo_id, node_id, version);
            let value = rmp_serde::to_vec(&v)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            let cf = cf_handle(&self.db, cf::VERSIONS)?;
            self.db
                .put_cf(cf, key, value)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        }

        Ok(())
    }
}
