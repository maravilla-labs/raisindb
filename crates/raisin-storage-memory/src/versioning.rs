//! In-memory versioning repository implementation

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use raisin_error::Result;
use raisin_models::nodes::{Node, NodeVersion};
use raisin_storage::VersioningRepository;

/// In-memory versioning repository
///
/// Stores node versions in memory using a HashMap.
/// Versions are lost when the process restarts.
#[derive(Clone, Default)]
pub struct InMemoryVersioningRepo {
    // key: node_id, value: ordered versions
    versions: Arc<RwLock<HashMap<String, Vec<NodeVersion>>>>,
}

impl VersioningRepository for InMemoryVersioningRepo {
    async fn create_version_with_note(&self, node: &Node, note: Option<String>) -> Result<i32> {
        let mut map = self.versions.write().await;
        let entry = map.entry(node.id.clone()).or_default();
        let ver = (entry.last().map(|v| v.version).unwrap_or(0)) + 1;
        entry.push(NodeVersion {
            id: format!("{}:{}", node.id, ver),
            node_id: node.id.clone(),
            version: ver,
            node_data: node.clone(),
            note,
            created_at: Some(chrono::Utc::now()),
            updated_at: Some(chrono::Utc::now()),
        });
        Ok(ver)
    }

    fn list_versions(
        &self,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<NodeVersion>>> + Send {
        let id = node_id.to_string();
        async move {
            let map = self.versions.read().await;
            Ok(map.get(&id).cloned().unwrap_or_default())
        }
    }

    fn get_version(
        &self,
        node_id: &str,
        version: i32,
    ) -> impl std::future::Future<Output = Result<Option<NodeVersion>>> + Send {
        let id = node_id.to_string();
        async move {
            let map = self.versions.read().await;
            Ok(map
                .get(&id)
                .and_then(|v| v.iter().find(|v| v.version == version).cloned()))
        }
    }

    fn delete_all_versions(
        &self,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<usize>> + Send {
        let id = node_id.to_string();
        async move {
            let mut map = self.versions.write().await;
            let count = map.get(&id).map(|v| v.len()).unwrap_or(0);
            map.remove(&id);
            Ok(count)
        }
    }

    async fn delete_version(&self, node_id: &str, version: i32) -> Result<bool> {
        let mut map = self.versions.write().await;

        if let Some(versions) = map.get_mut(node_id) {
            // Find the version to delete
            if let Some(pos) = versions.iter().position(|v| v.version == version) {
                let version_to_delete = &versions[pos];

                // Check if this version is published (cannot delete published versions)
                if version_to_delete.node_data.published_at.is_some() {
                    return Err(raisin_error::Error::Validation(
                        "Cannot delete published version".to_string(),
                    ));
                }

                // Remove the version
                versions.remove(pos);
                return Ok(true);
            }
        }

        Ok(false)
    }

    async fn delete_old_versions(&self, node_id: &str, keep_count: usize) -> Result<usize> {
        let mut map = self.versions.write().await;

        if let Some(versions) = map.get_mut(node_id) {
            if versions.len() <= keep_count {
                return Ok(0);
            }

            // Sort by version number (newest first)
            versions.sort_by(|a, b| b.version.cmp(&a.version));

            // Identify versions to delete (older than keep_count, and not published)
            let mut delete_indices = Vec::new();
            for (idx, version) in versions.iter().enumerate() {
                if idx >= keep_count && version.node_data.published_at.is_none() {
                    delete_indices.push(idx);
                }
            }

            // Delete in reverse order to maintain indices
            let count = delete_indices.len();
            for idx in delete_indices.into_iter().rev() {
                versions.remove(idx);
            }

            // Re-sort by version number for consistency
            versions.sort_by(|a, b| a.version.cmp(&b.version));

            return Ok(count);
        }

        Ok(0)
    }

    async fn update_version_note(
        &self,
        node_id: &str,
        version: i32,
        note: Option<String>,
    ) -> Result<()> {
        let mut map = self.versions.write().await;

        if let Some(versions) = map.get_mut(node_id) {
            if let Some(v) = versions.iter_mut().find(|v| v.version == version) {
                v.note = note;
                v.updated_at = Some(chrono::Utc::now());
                return Ok(());
            }
        }

        Err(raisin_error::Error::NotFound("version".to_string()))
    }
}
