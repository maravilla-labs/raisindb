//! How one node becomes archive entries, and its inventory line.
//!
//! The layout is the one the installer reads:
//!
//! - an asset with an embedded file: `content/{ws}{parent}.node.{name}.yaml`
//!   (metadata) and `content/{ws}{parent}{name}` (the bytes, streamed from
//!   the blob store in chunks, never held whole);
//! - a folder, or any node with children: `content/{ws}{path}/.node.yaml`;
//! - anything else: `content/{ws}{path}.yaml`;
//! - a node type: `nodetypes{path}.yaml`.
//!
//! Serialisation is canonical — every object's keys sorted — because node
//! properties live in a hash map, and an unstable key order would make the
//! same node hash differently on every build (every node "edited").

use super::inventory::{BinaryRef, InventoryLine};
use futures::StreamExt;
use raisin_binary::zip_stream::{Compression, ZipStreamWriter};
use raisin_binary::DynBinaryStorage;
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWrite;

/// Top-level node fields that are engine bookkeeping, not content: they are
/// left out of the inventory hash, so a node counts as edited only when a
/// restore would see a difference.
const BOOKKEEPING_FIELDS: &[&str] = &[
    "has_children",
    "children",
    "version",
    "updated_at",
    "updated_by",
    "published_at",
    "published_by",
    "tenant_id",
    "workspace",
];

/// Called after every binary chunk. Counting, cancellation, the in-flight
/// free-space check and record heartbeats all live behind it, so a single
/// multi-gigabyte file is as responsive as a page of small nodes.
pub(super) trait ChunkHook: Send {
    fn chunk(&mut self, n: u64) -> raisin_binary::BoxFuture<'_, Result<()>>;
}

/// What writing one node produced.
pub(super) struct Written {
    pub line: InventoryLine,
    /// Bytes written to the archive (uncompressed), entries included.
    pub bytes: u64,
    pub files: u64,
    /// The node names an embedded file that could not be read.
    pub missing_binary: Option<String>,
}

/// Sort every object's keys, recursively.
fn canonical(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(String, serde_json::Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k, canonical(v)))
                    .collect(),
            )
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(canonical).collect())
        }
        other => other,
    }
}

/// The node as archived: canonical YAML.
pub(super) fn serialize_node(node: &Node) -> Result<(String, serde_json::Value)> {
    let json = canonical(
        serde_json::to_value(node)
            .map_err(|e| Error::storage(format!("Failed to serialize node: {e}")))?,
    );
    let yaml = serde_yaml::to_string(&json)
        .map_err(|e| Error::storage(format!("Failed to convert node to YAML: {e}")))?;
    Ok((yaml, json))
}

/// `sha256:<hex>` over the archived form with bookkeeping stripped: the
/// fields in [`BOOKKEEPING_FIELDS`] and every engine-owned (`__`) property.
pub(super) fn content_hash(mut json: serde_json::Value) -> String {
    if let serde_json::Value::Object(map) = &mut json {
        for f in BOOKKEEPING_FIELDS {
            map.remove(*f);
        }
        if let Some(serde_json::Value::Object(props)) = map.get_mut("properties") {
            props.retain(|k, _| !k.starts_with("__"));
        }
    }
    let bytes = serde_json::to_vec(&json).unwrap_or_default();
    format!("sha256:{}", hex::encode(Sha256::digest(&bytes)))
}

fn is_folder_type(node_type: &str) -> bool {
    node_type.ends_with(":Folder") || node_type == "raisin:Folder"
}

/// `/a/b/c` → `/a/b/`, `/c` → `/`.
fn parent_path(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i + 1],
    }
}

/// The storage key and MIME type of a node's embedded file, if any.
pub(super) fn embedded_file(node: &Node) -> Option<(String, Option<String>)> {
    for key in ["file", "resource"] {
        match node.properties.get(key) {
            Some(PropertyValue::Resource(r)) => {
                if let Some(PropertyValue::String(k)) =
                    r.metadata.as_ref().and_then(|m| m.get("storage_key"))
                {
                    return Some((k.clone(), r.mime_type.clone()));
                }
            }
            Some(PropertyValue::Object(o)) => {
                let mime = match o.get("mime_type") {
                    Some(PropertyValue::String(m)) => Some(m.clone()),
                    _ => None,
                };
                if let Some(PropertyValue::Object(meta)) = o.get("metadata") {
                    if let Some(PropertyValue::String(k)) = meta.get("storage_key") {
                        return Some((k.clone(), mime));
                    }
                }
                if let Some(PropertyValue::String(k)) = o.get("key") {
                    return Some((k.clone(), mime));
                }
            }
            _ => {}
        }
    }
    None
}

/// Write one content node (and its embedded file) into the archive.
///
/// `has_children` is whether the node has children in the store; it decides
/// the folder layout for non-folder containers.
pub(super) async fn write_content_node<W: AsyncWrite + Unpin + Send>(
    zip: &mut ZipStreamWriter<W>,
    store: &dyn DynBinaryStorage,
    workspace: &str,
    node: &Node,
    has_children: bool,
    hook: &mut dyn ChunkHook,
) -> Result<Written> {
    let (yaml, json) = serialize_node(node)?;
    let hash = content_hash(json);
    let path = &node.path;
    let io = |e: std::io::Error| Error::storage(format!("Failed to write archive entry: {e}"));

    let mut bytes = yaml.len() as u64;
    let mut files = 1u64;
    let mut bin = None;
    let mut missing_binary = None;

    if let Some((storage_key, mime)) = embedded_file(node) {
        let parent = parent_path(path);
        let meta_entry = format!("content/{}{}.node.{}.yaml", workspace, parent, node.name);
        zip.write_entry(&meta_entry, Compression::Deflate(6), yaml.as_bytes())
            .await
            .map_err(io)?;
        match store.get_stream(&storage_key).await {
            Ok((_len, mut body)) => {
                let entry_name = format!("content/{}{}{}", workspace, parent, node.name);
                let mut entry = zip
                    .start_entry(&entry_name, Compression::for_content_type(mime.as_deref()))
                    .await
                    .map_err(io)?;
                while let Some(chunk) = body.next().await {
                    let chunk = chunk.map_err(|e| {
                        Error::storage(format!("Failed to read stored file of {path}: {e}"))
                    })?;
                    entry.write(&chunk).await.map_err(io)?;
                    hook.chunk(chunk.len() as u64).await?;
                }
                let digest = entry.finish().await.map_err(io)?;
                bytes += digest.size;
                files += 1;
                bin = Some(BinaryRef {
                    sha256: digest.sha256,
                    bytes: digest.size,
                });
            }
            Err(_) => missing_binary = Some(storage_key),
        }
    } else if is_folder_type(&node.node_type) || has_children {
        let entry = format!(
            "content/{}{}/.node.yaml",
            workspace,
            path.trim_end_matches('/')
        );
        zip.write_entry(&entry, Compression::Deflate(6), yaml.as_bytes())
            .await
            .map_err(io)?;
    } else {
        let entry = format!("content/{}{}.yaml", workspace, path);
        zip.write_entry(&entry, Compression::Deflate(6), yaml.as_bytes())
            .await
            .map_err(io)?;
    }

    Ok(Written {
        line: InventoryLine {
            ws: workspace.to_string(),
            path: path.clone(),
            id: node.id.clone(),
            node_type: node.node_type.clone(),
            hash,
            bytes: yaml.len() as u64,
            bin,
        },
        bytes,
        files,
        missing_binary,
    })
}

/// Write one node type definition.
pub(super) async fn write_node_type<W: AsyncWrite + Unpin + Send>(
    zip: &mut ZipStreamWriter<W>,
    node: &Node,
) -> Result<Written> {
    let (yaml, json) = serialize_node(node)?;
    let entry = format!("nodetypes{}.yaml", node.path);
    zip.write_entry(&entry, Compression::Deflate(6), yaml.as_bytes())
        .await
        .map_err(|e| Error::storage(format!("Failed to write archive entry: {e}")))?;
    Ok(Written {
        line: InventoryLine {
            ws: "nodetypes".to_string(),
            path: node.path.clone(),
            id: node.id.clone(),
            node_type: node.node_type.clone(),
            hash: content_hash(json),
            bytes: yaml.len() as u64,
            bin: None,
        },
        bytes: yaml.len() as u64,
        files: 1,
        missing_binary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn node_with(props: Vec<(&str, PropertyValue)>) -> Node {
        Node {
            id: "n1".into(),
            name: "home".into(),
            path: "/acme/home".into(),
            node_type: "studio:Page".into(),
            properties: props
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect::<HashMap<_, _>>(),
            ..Default::default()
        }
    }

    #[test]
    fn the_hash_ignores_key_order_and_bookkeeping() {
        let props: Vec<(&str, PropertyValue)> = (0..40)
            .map(|i| {
                (
                    Box::leak(format!("p{i}").into_boxed_str()) as &str,
                    PropertyValue::Integer(i),
                )
            })
            .collect();
        let a = node_with(props.clone());
        let mut rev = props.clone();
        rev.reverse();
        let mut b = node_with(rev);
        // Bookkeeping moves: not an edit.
        b.version = 7;
        b.updated_at = Some(chrono::Utc::now());
        b.has_children = Some(true);
        b.properties.insert(
            "__extract_status".into(),
            PropertyValue::String("ok".into()),
        );
        let h = |n: &Node| content_hash(serialize_node(n).unwrap().1);
        assert_eq!(h(&a), h(&b));
        // The archived YAML of the two content-equal nodes is byte-identical
        // where it matters: keys in the same order.
        let ya = serialize_node(&a).unwrap().0;
        assert!(ya.find("p0:").unwrap() < ya.find("p1:").unwrap());

        // A content change is an edit.
        let mut c = a.clone();
        c.properties
            .insert("title".into(), PropertyValue::String("New".into()));
        assert_ne!(h(&a), h(&c));
    }

    #[test]
    fn parent_paths() {
        assert_eq!(parent_path("/a/b/c"), "/a/b/");
        assert_eq!(parent_path("/c"), "/");
        assert_eq!(parent_path("c"), "/");
    }
}
