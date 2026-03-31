//! RevisionRepository trait implementation for RevisionRepositoryImpl.

use super::RevisionRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::tree::NodeChange;
use raisin_storage::{RevisionMeta, RevisionRepository};

impl RevisionRepository for RevisionRepositoryImpl {
    /// Allocate a new HLC revision (lock-free, ~23ns operation)
    fn allocate_revision(&self) -> HLC {
        self.hlc_state.tick()
    }

    async fn store_revision_meta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        meta: RevisionMeta,
    ) -> Result<()> {
        let key = keys::revision_meta_key(tenant_id, repo_id, &meta.revision);
        let value = rmp_serde::to_vec(&meta)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(())
    }

    async fn get_revision_meta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
    ) -> Result<Option<RevisionMeta>> {
        let key = keys::revision_meta_key(tenant_id, repo_id, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                let meta = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                Ok(Some(meta))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }

    async fn list_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RevisionMeta>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("revisions")
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut revisions = Vec::new();

        for (i, item) in iter.enumerate().skip(offset).take(limit) {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            let meta: RevisionMeta = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;
            revisions.push(meta);
        }

        Ok(revisions)
    }

    async fn list_changed_nodes(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
    ) -> Result<Vec<NodeChange>> {
        let meta = self
            .get_revision_meta(tenant_id, repo_id, revision)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::storage(format!("Revision {:?} not found", revision))
            })?;

        let mut changes = Vec::new();
        let cf_nodes = crate::cf_handle(&self.db, crate::cf::NODES)?;

        for change_info in &meta.changed_nodes {
            let node_key = crate::keys::node_key_versioned(
                tenant_id,
                repo_id,
                &meta.branch,
                &change_info.workspace,
                &change_info.node_id,
                revision,
            );

            let (path, node_type) = if let Some(value_bytes) =
                self.db
                    .get_cf(cf_nodes, &node_key)
                    .map_err(|e| raisin_error::Error::storage(e.to_string()))?
            {
                if !value_bytes.starts_with(b"TOMBSTONE") {
                    if let Ok(node) =
                        rmp_serde::from_slice::<raisin_models::nodes::Node>(&value_bytes)
                    {
                        (Some(node.path), Some(node.node_type))
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            changes.push(NodeChange {
                node_id: change_info.node_id.clone(),
                operation: change_info.operation,
                node_type,
                path,
                translation_locale: change_info.translation_locale.clone(),
            });
        }

        Ok(changes)
    }

    async fn index_node_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        node_id: &str,
    ) -> Result<()> {
        let key = keys::node_change_key(tenant_id, repo_id, node_id, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;

        self.db
            .put_cf(cf, key, b"")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(())
    }

    async fn index_node_type_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        node_type_name: &str,
    ) -> Result<()> {
        let key = keys::node_type_change_key(tenant_id, repo_id, node_type_name, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;

        self.db
            .put_cf(cf, key, b"")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(())
    }

    async fn index_archetype_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        archetype_name: &str,
    ) -> Result<()> {
        let key = keys::archetype_change_key(tenant_id, repo_id, archetype_name, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;

        self.db
            .put_cf(cf, key, b"")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(())
    }

    async fn index_element_type_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        element_type_name: &str,
    ) -> Result<()> {
        let key = keys::element_type_change_key(tenant_id, repo_id, element_type_name, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;

        self.db
            .put_cf(cf, key, b"")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(())
    }

    async fn get_node_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        limit: usize,
    ) -> Result<Vec<HLC>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("node_changes")
            .push(node_id)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut revisions = Vec::new();

        for (i, item) in iter.enumerate().take(limit) {
            let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            let key_str = String::from_utf8_lossy(&key);
            let parts: Vec<&str> = key_str.split('\0').collect();

            if let Some(rev_bytes) = parts.last() {
                if let Ok(rev) = keys::decode_descending_revision(rev_bytes.as_bytes()) {
                    revisions.push(rev);
                }
            }
        }

        Ok(revisions)
    }

    async fn get_node_type_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_type_name: &str,
        limit: usize,
    ) -> Result<Vec<HLC>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("node_type_changes")
            .push(node_type_name)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut revisions = Vec::new();

        for item in iter.take(limit) {
            let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let key_str = String::from_utf8_lossy(&key);
            if let Some(rev_part) = key_str.split('\0').next_back() {
                if let Ok(rev) = keys::decode_descending_revision(rev_part.as_bytes()) {
                    revisions.push(rev);
                }
            }
        }

        Ok(revisions)
    }

    async fn get_archetype_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        archetype_name: &str,
        limit: usize,
    ) -> Result<Vec<HLC>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("archetype_changes")
            .push(archetype_name)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut revisions = Vec::new();

        for item in iter.take(limit) {
            let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let key_str = String::from_utf8_lossy(&key);
            if let Some(rev_part) = key_str.split('\0').next_back() {
                if let Ok(rev) = keys::decode_descending_revision(rev_part.as_bytes()) {
                    revisions.push(rev);
                }
            }
        }

        Ok(revisions)
    }

    async fn get_element_type_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        element_type_name: &str,
        limit: usize,
    ) -> Result<Vec<HLC>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("element_type_changes")
            .push(element_type_name)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut revisions = Vec::new();

        for item in iter.take(limit) {
            let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let key_str = String::from_utf8_lossy(&key);
            if let Some(rev_part) = key_str.split('\0').next_back() {
                if let Ok(rev) = keys::decode_descending_revision(rev_part.as_bytes()) {
                    revisions.push(rev);
                }
            }
        }

        Ok(revisions)
    }

    async fn store_node_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        revision: &HLC,
        node_json: Vec<u8>,
    ) -> Result<()> {
        let key = keys::node_snapshot_key(tenant_id, repo_id, node_id, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;

        self.db
            .put_cf(cf, key, node_json)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(())
    }

    async fn get_node_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        revision: &HLC,
    ) -> Result<Option<Vec<u8>>> {
        let key = keys::node_snapshot_key(tenant_id, repo_id, node_id, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;

        self.db
            .get_cf(cf, key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))
    }

    async fn get_node_snapshot_at_or_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        revision: &HLC,
    ) -> Result<Option<(HLC, Vec<u8>)>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("snapshots")
            .push(node_id)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            let key_str = String::from_utf8_lossy(&key);
            let parts: Vec<&str> = key_str.split('\0').collect();

            if let Some(rev_bytes) = parts.last() {
                if let Ok(rev) = keys::decode_descending_revision(rev_bytes.as_bytes()) {
                    if &rev <= revision {
                        return Ok(Some((rev, value.to_vec())));
                    }
                }
            }
        }

        Ok(None)
    }

    async fn store_translation_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        locale: &str,
        revision: &HLC,
        overlay_json: Vec<u8>,
    ) -> Result<()> {
        let key = keys::translation_snapshot_key(tenant_id, repo_id, node_id, locale, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;

        self.db
            .put_cf(cf, key, overlay_json)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(())
    }

    async fn get_translation_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        locale: &str,
        revision: &HLC,
    ) -> Result<Option<Vec<u8>>> {
        let key = keys::translation_snapshot_key(tenant_id, repo_id, node_id, locale, revision);
        let cf = cf_handle(&self.db, cf::REVISIONS)?;

        self.db
            .get_cf(cf, key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))
    }

    async fn get_translation_snapshot_at_or_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        locale: &str,
        revision: &HLC,
    ) -> Result<Option<(HLC, Vec<u8>)>> {
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push("trans_snapshots")
            .push(node_id)
            .push(locale)
            .build_prefix();

        let cf = cf_handle(&self.db, cf::REVISIONS)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            let key_str = String::from_utf8_lossy(&key);
            let parts: Vec<&str> = key_str.split('\0').collect();

            if let Some(rev_bytes) = parts.last() {
                if let Ok(rev) = keys::decode_descending_revision(rev_bytes.as_bytes()) {
                    if &rev <= revision {
                        return Ok(Some((rev, value.to_vec())));
                    }
                }
            }
        }

        Ok(None)
    }
}
