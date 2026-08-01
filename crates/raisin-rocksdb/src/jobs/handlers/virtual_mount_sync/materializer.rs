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
    /// Re-materialize every item even when its etag is unchanged, and move it if
    /// the mapper/template now resolves a different path ("remap").
    ///
    /// Ordinary syncs must NOT do this: the etag skip-write is what stops a
    /// re-sync creating a revision per unchanged item and re-firing every
    /// downstream trigger. But that same skip returns before the mapper's output
    /// is applied, so a changed mapper — a new node type, a renamed property, a
    /// new folder hierarchy — is invisible to everything already synced. Remap
    /// is the deliberate, operator-triggered exception.
    pub force_rewrite: bool,
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
        let new_path = join_path(&scope.mount_path, rel_path);

        // A remap may re-derive a different path (a new folder hierarchy). The
        // relocation runs FIRST, in its own committed transaction, because a
        // move and a write cannot be combined in one: the in-transaction read
        // cache does not reflect the move, so writing after it collides on the
        // id, and moving after a write relocates the pre-write version and
        // discards the new mapping. Moving first leaves the ordinary upsert
        // below to find the node at its destination and update it in place.
        //
        // The node id is preserved throughout, so revision history and anything
        // added locally survive the migration — delete-and-recreate loses all of
        // it.
        if scope.force_rewrite {
            let tx = self.begin(scope, "virtual mount sync: remap move").await?;
            let all = tx.scan_nodes(&scope.workspace).await?;
            let found = all.into_iter().find(|n| {
                node_mount_id(n) == Some(scope.mount_id.as_str())
                    && node_external_id(n) == Some(virt.external_id.as_str())
                    && under(&scope.mount_path, &n.path)
            });
            if let Some(node) = found {
                if node.path != new_path {
                    ensure_folder_chain(tx.as_ref(), &scope.workspace, &new_path).await?;
                    tx.move_node_tree(&scope.workspace, &node.id, &new_path)
                        .await?;
                    tx.commit().await?;
                }
            }
        }

        let tx = self.begin(scope, "virtual mount sync: upsert").await?;

        // 1. Match by __external_id within the mount subtree (survives renames).
        let all = tx.scan_nodes(&scope.workspace).await?;
        let existing = all.into_iter().find(|n| {
            node_mount_id(n) == Some(scope.mount_id.as_str())
                && node_external_id(n) == Some(virt.external_id.as_str())
                && under(&scope.mount_path, &n.path)
        });

        let (id, path) = match existing {
            Some(node) => {
                // Etag skip-write: unchanged item → no revision churn. Bypassed
                // by a remap, which exists precisely to re-apply a mapper whose
                // output changed while the provider's item did not.
                if !scope.force_rewrite
                    && virt.etag.is_some()
                    && node_str_prop(&node, "__etag") == virt.etag
                {
                    return Ok(false);
                }
                // Normally: update in place, preserving id + current path (avoids
                // dupes on rename; a provider-side rename updates this node).
                //
                // On a remap the resolved path may have changed (a new folder
                // hierarchy), and it cannot simply be written: `upsert_node`
                // matches by PATH, so writing this id at a new path takes the
                // create branch and collides on the id. Move it explicitly,
                // which preserves the node id, its history and anything added
                // locally, then write at the new location.
                // A remap that re-derives a different path has already moved
                // the node (below, in its own committed transaction), so by now
                // `node.path` IS the destination.
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

/// Create any missing ancestor folders of `path`.
///
/// Only the MISSING ones: an `upsert_deep_node` on an existing folder matches by
/// path and would overwrite that folder's properties with an empty stub, which
/// on a conversation folder would wipe the thread subject the hierarchy exists
/// to display.
async fn ensure_folder_chain(
    tx: &dyn raisin_storage::transactional::TransactionalContext,
    workspace: &str,
    path: &str,
) -> Result<()> {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if segments.len() < 2 {
        return Ok(()); // no ancestors beyond the workspace root
    }
    let mut current = String::new();
    // All but the last segment: the last one is the node itself.
    for seg in &segments[..segments.len() - 1] {
        if seg.is_empty() {
            continue;
        }
        current.push('/');
        current.push_str(seg);
        if tx.get_node_by_path(workspace, &current).await?.is_some() {
            continue;
        }
        let folder = Node {
            id: nanoid::nanoid!(),
            node_type: "raisin:Folder".to_string(),
            name: (*seg).to_string(),
            path: current.clone(),
            workspace: Some(workspace.to_string()),
            ..Default::default()
        };
        tx.add_node(workspace, &folder).await?;
    }
    Ok(())
}
