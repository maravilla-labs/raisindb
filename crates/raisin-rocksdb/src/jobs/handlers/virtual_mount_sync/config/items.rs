//! The normalized item / change shapes adapters return, and the built-in
//! default mapping.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A normalized external object as returned by an adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalItem {
    pub external_id: String,
    pub name: String,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<i64>,
    #[serde(default)]
    pub is_folder: bool,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub modified_at: Option<String>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub web_url: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// One entry in a `get_changes` feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    #[serde(rename = "type")]
    pub kind: String,
    pub item: ExternalItem,
    #[serde(default)]
    pub relative_path: String,
}

/// A `get_changes` page.
#[derive(Debug, Clone, Deserialize)]
pub struct ChangesPage {
    #[serde(default)]
    pub items: Vec<Change>,
    #[serde(default)]
    pub next_token: Option<String>,
}

/// A `list` page.
#[derive(Debug, Clone, Deserialize)]
pub struct ListPage {
    #[serde(default)]
    pub items: Vec<ExternalItem>,
    #[serde(default)]
    pub next_cursor: Option<String>,
    /// Total items the provider says this listing has, when it knows.
    ///
    /// Optional because most paging APIs (Graph `@odata.nextLink`, IMAP, Drive)
    /// do not report one. Adapters that CAN report it turn the console's
    /// "1,240 imported" into "1,240 of 8,700" — so it is worth forwarding, but
    /// nothing may depend on its presence.
    #[serde(default)]
    pub total: Option<u64>,
}

/// The node shape a mapping produces (built-in default or a mapping function).
#[derive(Debug, Clone)]
pub struct MappedNode {
    pub node_type: String,
    pub name: Option<String>,
    pub properties: serde_json::Map<String, Value>,
}

/// Built-in Rust default mapping (§4.2): folder → `raisin:Folder`, everything
/// else → `raisin:Node` with title + a `meta` object carrying mime/size/urls and
/// provider passthrough. Zero function invocations.
///
/// Divergence from the frozen contract, which names `raisin:Asset`: that type
/// requires a binary `file` Resource, which a link-only v1 virtual node does not
/// have (see risk #7 "links only in v1"). `raisin:Node` is the correct permissive
/// carrier for metadata-only mounts; a mount that wants `raisin:Asset` (with real
/// content sync) supplies a custom `mapping_function`.
pub fn default_mapping(item: &ExternalItem) -> MappedNode {
    let mut props = serde_json::Map::new();
    if item.is_folder {
        MappedNode {
            node_type: "raisin:Folder".to_string(),
            name: Some(item.name.clone()),
            properties: props,
        }
    } else {
        props.insert("title".to_string(), Value::String(item.name.clone()));
        let mut meta = serde_json::Map::new();
        if let Some(mt) = &item.mime_type {
            meta.insert("mime_type".to_string(), Value::String(mt.clone()));
        }
        if let Some(sz) = item.size_bytes {
            meta.insert("size".to_string(), Value::from(sz));
        }
        if let Some(u) = &item.web_url {
            meta.insert("web_url".to_string(), Value::String(u.clone()));
        }
        if let Some(u) = &item.download_url {
            meta.insert("download_url".to_string(), Value::String(u.clone()));
        }
        if let Some(Value::Object(m)) = &item.metadata {
            for (k, v) in m {
                meta.insert(k.clone(), v.clone());
            }
        }
        if !meta.is_empty() {
            props.insert("meta".to_string(), Value::Object(meta));
        }
        MappedNode {
            node_type: "raisin:Node".to_string(),
            name: Some(item.name.clone()),
            properties: props,
        }
    }
}
