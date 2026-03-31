//! Reference index repository implementation

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_storage::scope::StorageScope;
use raisin_storage::ReferenceIndexRepository;
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct ReferenceIndexRepositoryImpl {
    db: Arc<DB>,
}

impl ReferenceIndexRepositoryImpl {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    fn extract_references(
        properties: &HashMap<String, PropertyValue>,
    ) -> Vec<(String, RaisinReference)> {
        let mut refs = Vec::new();

        fn visit_value(
            path: &str,
            value: &PropertyValue,
            refs: &mut Vec<(String, RaisinReference)>,
        ) {
            match value {
                PropertyValue::Reference(r) => {
                    refs.push((path.to_string(), r.clone()));
                }
                PropertyValue::Array(items) => {
                    for (i, item) in items.iter().enumerate() {
                        visit_value(&format!("{}.{}", path, i), item, refs);
                    }
                }
                PropertyValue::Object(obj) => {
                    for (key, val) in obj {
                        let new_path = if path.is_empty() {
                            key.clone()
                        } else {
                            format!("{}.{}", path, key)
                        };
                        visit_value(&new_path, val, refs);
                    }
                }
                _ => {}
            }
        }

        for (key, value) in properties {
            visit_value(key, value, &mut refs);
        }

        refs
    }
}

impl ReferenceIndexRepository for ReferenceIndexRepositoryImpl {
    async fn index_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        revision: &raisin_hlc::HLC,
        is_published: bool,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let refs = Self::extract_references(properties);
        let cf = cf_handle(&self.db, cf::REFERENCE_INDEX)?;

        for (property_path, reference) in refs {
            // Forward index (with revision)
            let forward_key = keys::reference_forward_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                node_id,
                &property_path,
                revision,
                is_published,
            );

            let value = rmp_serde::to_vec(&reference)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            self.db
                .put_cf(cf, forward_key, value)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Reverse index (with revision)
            let reverse_key = keys::reference_reverse_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &reference.workspace,
                &reference.path,
                node_id,
                &property_path,
                revision,
                is_published,
            );

            self.db
                .put_cf(cf, reverse_key, b"")
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        }

        Ok(())
    }

    async fn unindex_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        revision: &raisin_hlc::HLC,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        // Write tombstones for all references (MVCC-aware deletion)
        let refs = Self::extract_references(properties);
        let cf = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        const TOMBSTONE: &[u8] = b"T"; // Tombstone marker

        for (property_path, reference) in refs {
            // Tombstone forward index
            for is_pub in [false, true] {
                let forward_key = keys::reference_forward_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    node_id,
                    &property_path,
                    revision,
                    is_pub,
                );
                self.db
                    .put_cf(cf, forward_key, TOMBSTONE)
                    .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

                // Tombstone reverse index
                let reverse_key = keys::reference_reverse_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &reference.workspace,
                    &reference.path,
                    node_id,
                    &property_path,
                    revision,
                    is_pub,
                );
                self.db
                    .put_cf(cf, reverse_key, TOMBSTONE)
                    .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            }
        }

        Ok(())
    }

    async fn update_reference_publish_status(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        revision: &raisin_hlc::HLC,
        is_published: bool,
    ) -> Result<()> {
        // Tombstone old references, then write new ones at new revision
        self.unindex_references(scope, node_id, properties, revision)
            .await?;
        self.index_references(scope, node_id, properties, revision, is_published)
            .await?;
        Ok(())
    }

    async fn find_referencing_nodes(
        &self,
        scope: StorageScope<'_>,
        target_workspace: &str,
        target_path: &str,
        published_only: bool,
    ) -> Result<Vec<(String, String)>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let tag = if published_only {
            "ref_rev_pub"
        } else {
            "ref_rev"
        };

        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push(tag)
            .push(target_workspace)
            .push(target_path)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut results = Vec::new();

        for item in iter {
            let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let key_str = String::from_utf8_lossy(&key);
            let parts: Vec<&str> = key_str.split('\0').collect();

            if parts.len() >= 9 {
                let source_node_id = parts[7].to_string();
                let property_path = parts[8].to_string();
                results.push((source_node_id, property_path));
            }
        }

        Ok(results)
    }

    async fn get_node_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        published_only: bool,
    ) -> Result<Vec<(String, RaisinReference)>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let tag = if published_only { "ref_pub" } else { "ref" };

        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push(tag)
            .push(node_id)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut results = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let key_str = String::from_utf8_lossy(&key);
            let parts: Vec<&str> = key_str.split('\0').collect();

            if let Some(property_path) = parts.last() {
                let reference: RaisinReference = rmp_serde::from_slice(&value).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                results.push((property_path.to_string(), reference));
            }
        }

        Ok(results)
    }

    async fn get_unique_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        published_only: bool,
    ) -> Result<HashMap<String, (Vec<String>, RaisinReference)>> {
        let refs = self
            .get_node_references(scope, node_id, published_only)
            .await?;

        let mut unique = HashMap::new();

        for (property_path, reference) in refs {
            let key = format!("{}:{}", reference.workspace, reference.path);
            unique
                .entry(key)
                .or_insert_with(|| (Vec::new(), reference.clone()))
                .0
                .push(property_path);
        }

        Ok(unique)
    }
}
