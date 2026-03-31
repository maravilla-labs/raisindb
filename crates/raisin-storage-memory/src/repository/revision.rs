//! In-memory revision tracking implementation.

use crate::index_types::TranslationSnapshotIndex;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::{RevisionMeta, RevisionRepository};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory revision tracking
#[derive(Clone)]
pub struct InMemoryRevisionRepo {
    /// revisions: key = "{tenant_id}/{repo_id}/{revision}" -> RevisionMeta
    revisions: Arc<RwLock<HashMap<String, RevisionMeta>>>,

    /// revision_counter: key = "{tenant_id}/{repo_id}" -> next revision number
    #[allow(dead_code)]
    counters: Arc<RwLock<HashMap<String, u64>>>,

    /// node_changes: key = "{tenant_id}/{repo_id}/{node_id}" -> Vec<revision>
    node_changes: Arc<RwLock<HashMap<String, Vec<HLC>>>>,

    /// node_type_changes: key = "{tenant_id}/{repo_id}/{node_type_name}" -> Vec<revision>
    node_type_changes: Arc<RwLock<HashMap<String, Vec<HLC>>>>,

    /// archetype_changes: key = "{tenant_id}/{repo_id}/{archetype_name}" -> Vec<revision>
    archetype_changes: Arc<RwLock<HashMap<String, Vec<HLC>>>>,

    /// element_type_changes: key = "{tenant_id}/{repo_id}/{element_type_name}" -> Vec<revision>
    element_type_changes: Arc<RwLock<HashMap<String, Vec<HLC>>>>,

    /// translation snapshots keyed by tenant/repo/node/locale -> Vec<(revision, overlay)>
    translation_snapshots: TranslationSnapshotIndex,
}

impl Default for InMemoryRevisionRepo {
    fn default() -> Self {
        Self {
            revisions: Arc::new(RwLock::new(HashMap::new())),
            counters: Arc::new(RwLock::new(HashMap::new())),
            node_changes: Arc::new(RwLock::new(HashMap::new())),
            node_type_changes: Arc::new(RwLock::new(HashMap::new())),
            archetype_changes: Arc::new(RwLock::new(HashMap::new())),
            element_type_changes: Arc::new(RwLock::new(HashMap::new())),
            translation_snapshots: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl InMemoryRevisionRepo {
    fn make_revision_key(tenant_id: &str, repo_id: &str, revision: HLC) -> String {
        format!("{}/{}/{}", tenant_id, repo_id, revision)
    }

    fn make_node_key(tenant_id: &str, repo_id: &str, node_id: &str) -> String {
        format!("{}/{}/{}", tenant_id, repo_id, node_id)
    }

    fn make_node_type_key(tenant_id: &str, repo_id: &str, name: &str) -> String {
        format!("{}/{}/{}", tenant_id, repo_id, name)
    }

    fn make_archetype_key(tenant_id: &str, repo_id: &str, name: &str) -> String {
        format!("{}/{}/archetype/{}", tenant_id, repo_id, name)
    }

    fn make_element_type_key(tenant_id: &str, repo_id: &str, name: &str) -> String {
        format!("{}/{}/element_type/{}", tenant_id, repo_id, name)
    }

    fn make_translation_key(tenant_id: &str, repo_id: &str, node_id: &str, locale: &str) -> String {
        format!("{}/{}/{}/{}", tenant_id, repo_id, node_id, locale)
    }
}

impl RevisionRepository for InMemoryRevisionRepo {
    fn allocate_revision(&self) -> HLC {
        // For in-memory storage, we use a simple counter
        // In a real distributed system, this would use the actual HLC clock
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
        HLC::new(counter, 0)
    }

    async fn store_revision_meta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        meta: RevisionMeta,
    ) -> Result<()> {
        let key = Self::make_revision_key(tenant_id, repo_id, meta.revision);
        let mut revisions = self.revisions.write().await;
        revisions.insert(key, meta);
        Ok(())
    }

    async fn get_revision_meta(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
    ) -> Result<Option<RevisionMeta>> {
        let key = Self::make_revision_key(tenant_id, repo_id, *revision);
        let revisions = self.revisions.read().await;
        Ok(revisions.get(&key).cloned())
    }

    async fn list_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RevisionMeta>> {
        let prefix = format!("{}/{}/", tenant_id, repo_id);
        let revisions = self.revisions.read().await;

        let mut repo_revisions: Vec<RevisionMeta> = revisions
            .iter()
            .filter_map(|(key, meta)| {
                if key.starts_with(&prefix) {
                    Some(meta.clone())
                } else {
                    None
                }
            })
            .collect();

        // Sort by revision number (newest first)
        repo_revisions.sort_by(|a, b| b.revision.cmp(&a.revision));

        // Apply offset and limit
        Ok(repo_revisions
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect())
    }

    async fn list_changed_nodes(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _revision: &HLC,
    ) -> Result<Vec<raisin_models::tree::NodeChange>> {
        // For memory storage (test mode), return empty list for now
        // In a real implementation, this would need tree diff like RocksDB version
        // For tests that need change tracking, they should use the RocksDB backend
        Ok(Vec::new())
    }

    async fn index_node_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        node_id: &str,
    ) -> Result<()> {
        let key = Self::make_node_key(tenant_id, repo_id, node_id);
        let mut node_changes = self.node_changes.write().await;

        node_changes
            .entry(key)
            .or_insert_with(Vec::new)
            .push(*revision);

        Ok(())
    }

    async fn index_node_type_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        node_type_name: &str,
    ) -> Result<()> {
        let key = Self::make_node_type_key(tenant_id, repo_id, node_type_name);
        let mut node_type_changes = self.node_type_changes.write().await;

        node_type_changes
            .entry(key)
            .or_insert_with(Vec::new)
            .push(*revision);

        Ok(())
    }

    async fn index_archetype_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        archetype_name: &str,
    ) -> Result<()> {
        let key = Self::make_archetype_key(tenant_id, repo_id, archetype_name);
        let mut archetype_changes = self.archetype_changes.write().await;

        archetype_changes
            .entry(key)
            .or_insert_with(Vec::new)
            .push(*revision);

        Ok(())
    }

    async fn index_element_type_change(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: &HLC,
        element_type_name: &str,
    ) -> Result<()> {
        let key = Self::make_element_type_key(tenant_id, repo_id, element_type_name);
        let mut element_type_changes = self.element_type_changes.write().await;

        element_type_changes
            .entry(key)
            .or_insert_with(Vec::new)
            .push(*revision);

        Ok(())
    }

    async fn get_node_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        limit: usize,
    ) -> Result<Vec<HLC>> {
        let key = Self::make_node_key(tenant_id, repo_id, node_id);
        let node_changes = self.node_changes.read().await;

        let mut revisions = node_changes.get(&key).cloned().unwrap_or_default();

        // Sort newest first
        revisions.sort_by(|a, b| b.cmp(a));

        // Apply limit
        Ok(revisions.into_iter().take(limit).collect())
    }

    async fn get_node_type_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_type_name: &str,
        limit: usize,
    ) -> Result<Vec<HLC>> {
        let key = Self::make_node_type_key(tenant_id, repo_id, node_type_name);
        let node_type_changes = self.node_type_changes.read().await;

        let mut revisions = node_type_changes.get(&key).cloned().unwrap_or_default();

        revisions.sort_by(|a, b| b.cmp(a));
        Ok(revisions.into_iter().take(limit).collect())
    }

    async fn get_archetype_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        archetype_name: &str,
        limit: usize,
    ) -> Result<Vec<HLC>> {
        let key = Self::make_archetype_key(tenant_id, repo_id, archetype_name);
        let archetype_changes = self.archetype_changes.read().await;

        let mut revisions = archetype_changes.get(&key).cloned().unwrap_or_default();
        revisions.sort_by(|a, b| b.cmp(a));
        Ok(revisions.into_iter().take(limit).collect())
    }

    async fn get_element_type_revisions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        element_type_name: &str,
        limit: usize,
    ) -> Result<Vec<HLC>> {
        let key = Self::make_element_type_key(tenant_id, repo_id, element_type_name);
        let element_type_changes = self.element_type_changes.read().await;

        let mut revisions = element_type_changes.get(&key).cloned().unwrap_or_default();
        revisions.sort_by(|a, b| b.cmp(a));
        Ok(revisions.into_iter().take(limit).collect())
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
        let key = Self::make_translation_key(tenant_id, repo_id, node_id, locale);
        let mut snapshots = self.translation_snapshots.write().await;
        let entry = snapshots.entry(key).or_insert_with(Vec::new);

        entry.push((*revision, overlay_json));
        entry.sort_by(|a, b| b.0.cmp(&a.0));

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
        let key = Self::make_translation_key(tenant_id, repo_id, node_id, locale);
        let snapshots = self.translation_snapshots.read().await;

        Ok(snapshots.get(&key).and_then(|entries| {
            entries
                .iter()
                .find(|(rev, _)| rev == revision)
                .map(|(_, data)| data.clone())
        }))
    }

    async fn get_translation_snapshot_at_or_before(
        &self,
        tenant_id: &str,
        repo_id: &str,
        node_id: &str,
        locale: &str,
        revision: &HLC,
    ) -> Result<Option<(HLC, Vec<u8>)>> {
        let key = Self::make_translation_key(tenant_id, repo_id, node_id, locale);
        let snapshots = self.translation_snapshots.read().await;

        let result = snapshots.get(&key).and_then(|entries| {
            entries
                .iter()
                .find(|(rev, _)| rev <= revision)
                .map(|(rev, data)| (*rev, data.clone()))
        });

        Ok(result)
    }

    async fn store_node_snapshot(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _node_id: &str,
        _revision: &HLC,
        _node_json: Vec<u8>,
    ) -> Result<()> {
        // In-memory storage doesn't implement revision snapshots yet
        // This is a stub to satisfy the trait
        Ok(())
    }

    async fn get_node_snapshot(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _node_id: &str,
        _revision: &HLC,
    ) -> Result<Option<Vec<u8>>> {
        // In-memory storage doesn't implement revision snapshots yet
        Ok(None)
    }

    async fn get_node_snapshot_at_or_before(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _node_id: &str,
        _revision: &HLC,
    ) -> Result<Option<(HLC, Vec<u8>)>> {
        // In-memory storage doesn't implement revision snapshots yet
        Ok(None)
    }
}
