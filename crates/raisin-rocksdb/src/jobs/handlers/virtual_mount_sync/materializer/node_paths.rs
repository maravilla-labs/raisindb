//! Reserved-property readers, path arithmetic, and the error classification that
//! makes skip-and-continue safe.

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalContext;

/// Whether an error rejects ONE item rather than poisoning the transaction.
///
/// This distinction is what makes skip-and-continue safe: in `add_node` and
/// `put_node` every check that produces one of these runs BEFORE the first batch
/// write, so a rejected item contributed nothing to the shared `WriteBatch`.
pub fn is_item_level(error: &Error) -> bool {
    matches!(
        error,
        Error::NotFound(_)
            | Error::AlreadyExists(_)
            | Error::Validation(_)
            | Error::Conflict(_)
            | Error::Unauthorized(_)
            | Error::Forbidden(_)
            | Error::PermissionDenied(_)
    )
}

/// Read `__mount_id` from a node.
pub(crate) fn node_mount_id(node: &Node) -> Option<&str> {
    match node.properties.get("__mount_id")? {
        PropertyValue::String(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Read `__external_id` from a node.
pub(crate) fn node_external_id(node: &Node) -> Option<&str> {
    match node.properties.get("__external_id")? {
        PropertyValue::String(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Read a `__`-prefixed string property.
pub(crate) fn node_str_prop(node: &Node, key: &str) -> Option<String> {
    match node.properties.get(key)? {
        PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Read `__synced_at` as unix epoch seconds. The storage layer may coerce an
/// ISO string into a `Date`, so accept both.
pub(super) fn node_synced_secs(node: &Node) -> Option<i64> {
    match node.properties.get("__synced_at")? {
        PropertyValue::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.timestamp()),
        PropertyValue::Date(d) => Some(d.timestamp()),
        _ => None,
    }
}

/// Whether `path` is at or under `prefix`.
pub(crate) fn under(prefix: &str, path: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

/// Join the mount path with a relative path into an absolute node path.
pub(super) fn join_path(mount_path: &str, rel: &str) -> String {
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

/// Every ancestor path of `path`, excluding `path` itself and the root.
pub(super) fn ancestor_paths(path: &str) -> Vec<String> {
    let segments: Vec<&str> = path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for seg in &segments[..segments.len().saturating_sub(1)] {
        current.push('/');
        current.push_str(seg);
        out.push(current.clone());
    }
    out
}

/// Create any missing ancestor folders of `path`.
///
/// Only the MISSING ones: an `upsert_deep_node` on an existing folder matches by
/// path and would overwrite that folder's properties with an empty stub, which
/// on a conversation folder would wipe the thread subject the hierarchy exists
/// to display.
pub(super) async fn ensure_folder_chain(
    tx: &dyn TransactionalContext,
    workspace: &str,
    path: &str,
) -> Result<()> {
    for ancestor in ancestor_paths(path) {
        if tx.get_node_by_path(workspace, &ancestor).await?.is_some() {
            continue;
        }
        let name = ancestor.rsplit('/').next().unwrap_or("folder").to_string();
        let folder = Node {
            id: nanoid::nanoid!(),
            node_type: "raisin:Folder".to_string(),
            name,
            path: ancestor,
            workspace: Some(workspace.to_string()),
            ..Default::default()
        };
        tx.add_node(workspace, &folder).await?;
    }
    Ok(())
}
