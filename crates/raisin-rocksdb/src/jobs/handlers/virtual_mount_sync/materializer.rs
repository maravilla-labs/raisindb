//! Transactional materialization of external items into nodes.
//!
//! Every write goes through the normal transactional write path under actor
//! [`SYNC_ACTOR`] with a system auth context, so `node_event` triggers,
//! fulltext, SQL indexes, audit, and replication all apply for free.

use async_trait::async_trait;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalStorage;
use std::sync::Arc;

use super::config::{build_properties, MappedNode, SYNC_ACTOR};
use crate::RocksDBStorage;

/// Identifies one mount within a repo/branch/workspace.
#[derive(Debug, Clone)]
pub struct MountScope {
    pub tenant: String,
    pub repo: String,
    pub branch: String,
    /// Target workspace the mount materializes into.
    pub workspace: String,
    pub mount_id: String,
    /// Path prefix inside the target workspace (e.g. `/documents/shared`).
    pub mount_path: String,
}

/// Reserved virtual metadata stamped on every synced node.
#[derive(Debug, Clone)]
pub struct VirtualMeta {
    pub mount_id: String,
    pub external_id: String,
    pub etag: Option<String>,
    /// ISO 8601 timestamp of this sync write.
    pub synced_at: String,
}

/// Lightweight reference to an existing mount-owned node.
#[derive(Debug, Clone)]
pub struct VirtualNodeRef {
    pub id: String,
    pub path: String,
    pub external_id: String,
    pub etag: Option<String>,
    /// `__synced_at` as unix epoch seconds (used by ephemeral TTL cleanup).
    /// Tolerates both `String` (ISO 8601) and `Date` stored representations.
    pub synced_secs: Option<i64>,
}

/// Materializes mapped external items into nodes. Deletes are scoped to the
/// mount — user-created nodes under the mount path are never touched.
#[async_trait]
pub trait NodeMaterializer: Send + Sync {
    /// Upsert one item at `rel_path` under the mount. Returns `true` if a write
    /// happened, `false` if it was skipped (etag match or a foreign node blocks
    /// the path).
    async fn upsert(
        &self,
        scope: &MountScope,
        rel_path: &str,
        mapped: MappedNode,
        virt: VirtualMeta,
    ) -> Result<bool>;

    /// Delete the mount-owned node with the given `external_id`. Returns `true`
    /// if a node was removed. Never removes a node whose `__mount_id` differs.
    async fn delete(&self, scope: &MountScope, external_id: &str) -> Result<bool>;

    /// List all mount-owned virtual nodes (for full reconcile and TTL cleanup).
    async fn list_virtual(&self, scope: &MountScope) -> Result<Vec<VirtualNodeRef>>;
}

/// RocksDB-backed materializer.
pub struct RocksDbMaterializer {
    storage: Arc<RocksDBStorage>,
}

impl RocksDbMaterializer {
    /// Create a materializer bound to storage.
    pub fn new(storage: Arc<RocksDBStorage>) -> Self {
        Self { storage }
    }

    /// Open a transaction scoped to the mount with the sync actor + system auth.
    async fn begin(
        &self,
        scope: &MountScope,
        message: &str,
    ) -> Result<Box<dyn raisin_storage::transactional::TransactionalContext>> {
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(&scope.tenant, &scope.repo)?;
        tx.set_branch(&scope.branch)?;
        tx.set_actor(SYNC_ACTOR)?;
        tx.set_auth_context(AuthContext::system())?;
        tx.set_message(message)?;
        Ok(tx)
    }
}

/// Read `__mount_id` from a node.
fn node_mount_id(node: &Node) -> Option<&str> {
    match node.properties.get("__mount_id")? {
        PropertyValue::String(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Read `__external_id` from a node.
fn node_external_id(node: &Node) -> Option<&str> {
    match node.properties.get("__external_id")? {
        PropertyValue::String(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Read a `__`-prefixed string property.
fn node_str_prop(node: &Node, key: &str) -> Option<String> {
    match node.properties.get(key)? {
        PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Read `__synced_at` as unix epoch seconds. The storage layer may coerce an
/// ISO string into a `Date`, so accept both.
fn node_synced_secs(node: &Node) -> Option<i64> {
    match node.properties.get("__synced_at")? {
        PropertyValue::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.timestamp()),
        PropertyValue::Date(d) => Some(d.timestamp()),
        _ => None,
    }
}

/// Whether `path` is at or under `prefix`.
fn under(prefix: &str, path: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

/// Join the mount path with a relative path into an absolute node path.
fn join_path(mount_path: &str, rel: &str) -> String {
    let rel = rel.trim_start_matches('/');
    if rel.is_empty() {
        return mount_path.to_string();
    }
    if mount_path == "/" {
        format!("/{rel}")
    } else {
        format!("{mount_path}/{rel}")
    }
}

#[async_trait]
impl NodeMaterializer for RocksDbMaterializer {
    async fn upsert(
        &self,
        scope: &MountScope,
        rel_path: &str,
        mapped: MappedNode,
        virt: VirtualMeta,
    ) -> Result<bool> {
        let tx = self.begin(scope, "virtual mount sync: upsert").await?;

        // 1. Match by __external_id within the mount subtree (survives renames).
        let all = tx.scan_nodes(&scope.workspace).await?;
        let existing = all.into_iter().find(|n| {
            node_mount_id(n) == Some(scope.mount_id.as_str())
                && node_external_id(n) == Some(virt.external_id.as_str())
                && under(&scope.mount_path, &n.path)
        });

        let new_path = join_path(&scope.mount_path, rel_path);

        let (id, path) = match existing {
            Some(node) => {
                // Etag skip-write: unchanged item → no revision churn.
                if virt.etag.is_some() && node_str_prop(&node, "__etag") == virt.etag {
                    return Ok(false);
                }
                // Update in place, preserving id + current path (avoids dupes on
                // rename; a provider-side rename updates this node).
                (node.id, node.path)
            }
            None => {
                // Fall back to a path match. A foreign (non-mount) node sitting
                // at the target path must not be clobbered.
                if let Some(at_path) = tx.get_node_by_path(&scope.workspace, &new_path).await? {
                    if node_mount_id(&at_path) != Some(scope.mount_id.as_str()) {
                        tracing::warn!(
                            path = %new_path,
                            "virtual mount upsert: foreign node occupies target path, skipping"
                        );
                        return Ok(false);
                    }
                    if virt.etag.is_some() && node_str_prop(&at_path, "__etag") == virt.etag {
                        return Ok(false);
                    }
                    (at_path.id, at_path.path)
                } else {
                    (nanoid::nanoid!(), new_path)
                }
            }
        };

        let name = mapped
            .name
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| path.rsplit('/').next().unwrap_or("item").to_string());

        let node = Node {
            id,
            node_type: mapped.node_type.clone(),
            name,
            path,
            workspace: Some(scope.workspace.clone()),
            properties: build_properties(&mapped.properties, &virt),
            ..Default::default()
        };

        // Deep upsert auto-creates any missing parent folders.
        tx.upsert_deep_node(&scope.workspace, &node, "raisin:Folder")
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn delete(&self, scope: &MountScope, external_id: &str) -> Result<bool> {
        let tx = self.begin(scope, "virtual mount sync: delete").await?;
        let all = tx.scan_nodes(&scope.workspace).await?;
        let target = all.into_iter().find(|n| {
            node_mount_id(n) == Some(scope.mount_id.as_str())
                && node_external_id(n) == Some(external_id)
                && under(&scope.mount_path, &n.path)
        });
        match target {
            Some(node) => {
                tx.delete_node(&scope.workspace, &node.id).await?;
                tx.commit().await?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn list_virtual(&self, scope: &MountScope) -> Result<Vec<VirtualNodeRef>> {
        let tx = self.begin(scope, "virtual mount sync: list").await?;
        let all = tx.scan_nodes(&scope.workspace).await?;
        let refs = all
            .into_iter()
            .filter(|n| {
                node_mount_id(n) == Some(scope.mount_id.as_str())
                    && under(&scope.mount_path, &n.path)
            })
            .filter_map(|n| {
                node_external_id(&n).map(|ext| VirtualNodeRef {
                    id: n.id.clone(),
                    path: n.path.clone(),
                    external_id: ext.to_string(),
                    etag: node_str_prop(&n, "__etag"),
                    synced_secs: node_synced_secs(&n),
                })
            })
            .collect();
        Ok(refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_and_under_paths() {
        assert_eq!(join_path("/docs", "a/b"), "/docs/a/b");
        assert_eq!(join_path("/docs", "/a"), "/docs/a");
        assert_eq!(join_path("/", "a"), "/a");
        assert!(under("/docs", "/docs/a"));
        assert!(under("/docs", "/docs"));
        assert!(!under("/docs", "/documents"));
        assert!(under("/", "/anything"));
    }
}
