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

//! RocksDB implementation of SystemUpdateRepository
//!
//! Tracks which versions of built-in NodeTypes and Workspaces have been
//! applied to each repository.

use crate::cf;
use raisin_error::Result;
use raisin_storage::system_updates::{AppliedDefinition, ResourceType, SystemUpdateRepository};
use rocksdb::DB;
use std::sync::Arc;

/// RocksDB implementation of SystemUpdateRepository
#[derive(Clone)]
pub struct SystemUpdateRepositoryImpl {
    db: Arc<DB>,
}

impl SystemUpdateRepositoryImpl {
    /// Create a new SystemUpdateRepositoryImpl
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Build the key for storing an applied definition
    ///
    /// Key format: {tenant_id}:{repo_id}:{resource_type}:{name}
    fn make_key(
        tenant_id: &str,
        repo_id: &str,
        resource_type: ResourceType,
        name: &str,
    ) -> Vec<u8> {
        format!("{}:{}:{}:{}", tenant_id, repo_id, resource_type, name).into_bytes()
    }

    /// Build the prefix for listing all applied definitions for a repository
    ///
    /// Prefix format: {tenant_id}:{repo_id}:
    fn make_prefix(tenant_id: &str, repo_id: &str) -> Vec<u8> {
        format!("{}:{}:", tenant_id, repo_id).into_bytes()
    }

    /// Parse a key to extract resource type and name
    fn parse_key(key: &[u8]) -> Option<(ResourceType, String)> {
        let key_str = std::str::from_utf8(key).ok()?;
        let parts: Vec<&str> = key_str.split(':').collect();
        if parts.len() < 4 {
            return None;
        }
        // Format: tenant:repo:type:name (name may contain colons)
        let resource_type: ResourceType = parts[2].parse().ok()?;
        let name = parts[3..].join(":");
        Some((resource_type, name))
    }
}

#[async_trait::async_trait]
impl SystemUpdateRepository for SystemUpdateRepositoryImpl {
    async fn get_applied(
        &self,
        tenant_id: &str,
        repo_id: &str,
        resource_type: ResourceType,
        name: &str,
    ) -> Result<Option<AppliedDefinition>> {
        let cf = self.db.cf_handle(cf::SYSTEM_UPDATE_HASHES).ok_or_else(|| {
            raisin_error::Error::storage("SYSTEM_UPDATE_HASHES column family not found")
        })?;

        let key = Self::make_key(tenant_id, repo_id, resource_type, name);

        match self
            .db
            .get_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(format!("RocksDB get error: {}", e)))?
        {
            Some(data) => {
                let entry: AppliedDefinition = rmp_serde::from_slice(&data).map_err(|e| {
                    raisin_error::Error::storage(format!(
                        "Failed to deserialize AppliedDefinition: {}",
                        e
                    ))
                })?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    async fn set_applied(
        &self,
        tenant_id: &str,
        repo_id: &str,
        resource_type: ResourceType,
        name: &str,
        entry: AppliedDefinition,
    ) -> Result<()> {
        let cf = self.db.cf_handle(cf::SYSTEM_UPDATE_HASHES).ok_or_else(|| {
            raisin_error::Error::storage("SYSTEM_UPDATE_HASHES column family not found")
        })?;

        let key = Self::make_key(tenant_id, repo_id, resource_type, name);
        let data = rmp_serde::to_vec(&entry).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize AppliedDefinition: {}", e))
        })?;

        self.db
            .put_cf(cf, &key, &data)
            .map_err(|e| raisin_error::Error::storage(format!("RocksDB put error: {}", e)))?;

        tracing::debug!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            resource_type = %resource_type,
            name = %name,
            content_hash = %entry.content_hash,
            "Recorded applied system definition"
        );

        Ok(())
    }

    async fn list_applied(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<Vec<(ResourceType, String, AppliedDefinition)>> {
        let cf = self.db.cf_handle(cf::SYSTEM_UPDATE_HASHES).ok_or_else(|| {
            raisin_error::Error::storage("SYSTEM_UPDATE_HASHES column family not found")
        })?;

        let prefix = Self::make_prefix(tenant_id, repo_id);
        let mut results = Vec::new();

        let iter = self.db.prefix_iterator_cf(cf, &prefix);
        for item in iter {
            let (key, value) = item.map_err(|e| {
                raisin_error::Error::storage(format!("RocksDB iterator error: {}", e))
            })?;

            // Stop when we've moved past our prefix
            if !key.starts_with(&prefix) {
                break;
            }

            if let Some((resource_type, name)) = Self::parse_key(&key) {
                match rmp_serde::from_slice::<AppliedDefinition>(&value) {
                    Ok(entry) => {
                        results.push((resource_type, name, entry));
                    }
                    Err(e) => {
                        tracing::warn!(
                            key = ?String::from_utf8_lossy(&key),
                            error = %e,
                            "Failed to deserialize AppliedDefinition, skipping"
                        );
                    }
                }
            }
        }

        Ok(results)
    }

    async fn delete_applied(
        &self,
        tenant_id: &str,
        repo_id: &str,
        resource_type: ResourceType,
        name: &str,
    ) -> Result<()> {
        let cf = self.db.cf_handle(cf::SYSTEM_UPDATE_HASHES).ok_or_else(|| {
            raisin_error::Error::storage("SYSTEM_UPDATE_HASHES column family not found")
        })?;

        let key = Self::make_key(tenant_id, repo_id, resource_type, name);
        self.db
            .delete_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(format!("RocksDB delete error: {}", e)))?;

        tracing::debug!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            resource_type = %resource_type,
            name = %name,
            "Deleted applied system definition record"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    fn create_test_db() -> Arc<DB> {
        let dir = tempdir().unwrap();
        let db = crate::open_db(dir.path()).unwrap();
        Arc::new(db)
    }

    #[tokio::test]
    async fn test_set_and_get_applied() {
        let db = create_test_db();
        let repo = SystemUpdateRepositoryImpl::new(db);

        let entry = AppliedDefinition {
            content_hash: "abc123".to_string(),
            applied_version: Some(1),
            applied_at: Utc::now(),
            applied_by: "system".to_string(),
        };

        // Set applied
        repo.set_applied(
            "tenant1",
            "repo1",
            ResourceType::NodeType,
            "raisin:Folder",
            entry.clone(),
        )
        .await
        .unwrap();

        // Get applied
        let result = repo
            .get_applied("tenant1", "repo1", ResourceType::NodeType, "raisin:Folder")
            .await
            .unwrap();

        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.content_hash, "abc123");
        assert_eq!(retrieved.applied_version, Some(1));
        assert_eq!(retrieved.applied_by, "system");
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let db = create_test_db();
        let repo = SystemUpdateRepositoryImpl::new(db);

        let result = repo
            .get_applied("tenant1", "repo1", ResourceType::NodeType, "nonexistent")
            .await
            .unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_list_applied() {
        let db = create_test_db();
        let repo = SystemUpdateRepositoryImpl::new(db);

        // Add several entries
        let now = Utc::now();
        for (resource_type, name) in [
            (ResourceType::NodeType, "raisin:Folder"),
            (ResourceType::NodeType, "raisin:Page"),
            (ResourceType::Workspace, "default"),
        ] {
            let entry = AppliedDefinition {
                content_hash: format!("hash-{}", name),
                applied_version: Some(1),
                applied_at: now,
                applied_by: "system".to_string(),
            };
            repo.set_applied("tenant1", "repo1", resource_type, name, entry)
                .await
                .unwrap();
        }

        // Add entry for different repo (should not be returned)
        let entry = AppliedDefinition {
            content_hash: "other-hash".to_string(),
            applied_version: Some(1),
            applied_at: now,
            applied_by: "system".to_string(),
        };
        repo.set_applied(
            "tenant1",
            "repo2",
            ResourceType::NodeType,
            "raisin:Asset",
            entry,
        )
        .await
        .unwrap();

        // List for repo1
        let results = repo.list_applied("tenant1", "repo1").await.unwrap();

        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn test_delete_applied() {
        let db = create_test_db();
        let repo = SystemUpdateRepositoryImpl::new(db);

        let entry = AppliedDefinition {
            content_hash: "abc123".to_string(),
            applied_version: Some(1),
            applied_at: Utc::now(),
            applied_by: "system".to_string(),
        };

        // Set applied
        repo.set_applied(
            "tenant1",
            "repo1",
            ResourceType::NodeType,
            "raisin:Folder",
            entry,
        )
        .await
        .unwrap();

        // Verify it exists
        let result = repo
            .get_applied("tenant1", "repo1", ResourceType::NodeType, "raisin:Folder")
            .await
            .unwrap();
        assert!(result.is_some());

        // Delete
        repo.delete_applied("tenant1", "repo1", ResourceType::NodeType, "raisin:Folder")
            .await
            .unwrap();

        // Verify it's gone
        let result = repo
            .get_applied("tenant1", "repo1", ResourceType::NodeType, "raisin:Folder")
            .await
            .unwrap();
        assert!(result.is_none());
    }
}
