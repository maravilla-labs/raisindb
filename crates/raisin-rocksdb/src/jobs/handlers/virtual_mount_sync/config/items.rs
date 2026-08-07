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
    /// Whether the provider has MORE pages right now (`true`: `next_token` is
    /// a mid-enumeration cursor, keep paging) or is caught up (`false`:
    /// `next_token` is the resume point for the NEXT run — stop).
    ///
    /// The adapter must say this explicitly because token identity cannot:
    /// Microsoft Graph mints a fresh delta token on every poll of an idle
    /// feed, so "the token stopped changing" NEVER holds there, and the delta
    /// loop span empty pages at ~4 requests/second — committing the fresh
    /// cursor each time — until the watchdog killed the run. `None` (legacy
    /// adapter without the field) falls back to the empty-page heuristic in
    /// `delta.rs`.
    #[serde(default)]
    pub has_more: Option<bool>,
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

/// What one `to_node` invocation produced: the item's own node, plus any
/// subordinate nodes the mapper attached beneath it.
///
/// `children` is separate from [`MappedNode`] rather than a field on it because
/// `MappedNode` is what a [`BatchOp`](super::super::materializer::BatchOp)
/// carries — one node, one write. A child is its own `BatchOp`, with its own
/// `__external_id` and its own row in the sync index, so folding the two shapes
/// together would put a value in the batch that the batch cannot act on.
#[derive(Debug, Clone)]
pub struct MappedItem {
    pub node: MappedNode,
    /// Empty for every mapper written before this existed, which is why the
    /// feature costs a read-only mount nothing.
    pub children: Vec<MappedChild>,
}

/// Separator between a parent item's `external_id` and a child's own id.
///
/// A child id is namespaced because it is only unique WITHIN its parent: a Graph
/// attachment id repeats across messages, and an IMAP MIME part number is
/// literally "2" on every message that has one. Without the parent prefix the
/// second message's attachment would match the first's node by `__external_id`
/// and the sync would relocate one node back and forth forever.
///
/// The separator is a byte no provider id contains rather than a punctuation
/// character one might: `#` and `:` both appear in real Graph ids.
pub const CHILD_ID_SEP: &str = "\u{1f}";

/// Namespaced `external_id` for a child of `parent_external_id`.
pub fn child_external_id(parent_external_id: &str, child_id: &str) -> String {
    format!("{parent_external_id}{CHILD_ID_SEP}{child_id}")
}

/// One subordinate node attached to a mapped item.
///
/// Deliberately NOT an array property on the parent. The Drive adapter already
/// maps provider blobs to `raisin:Asset`, so an array would build a second,
/// parallel blob path — this codebase's most expensive recurring bug class — and
/// an array element cannot carry a binary-store reference, be fulltext-indexed,
/// or be fetched on demand without inventing a per-element addressing scheme
/// that nodes already have.
#[derive(Debug, Clone)]
pub struct MappedChild {
    /// Path segment under the parent node. Also the child node's name.
    pub name: String,
    pub node_type: String,
    /// The provider's id for this child, BEFORE namespacing. Unique within the
    /// parent only; [`child_external_id`] makes it unique within the mount.
    pub external_id: String,
    /// Change token, if the provider has one for the child itself. Absent is
    /// normal: attachments of an immutable message do not change independently,
    /// so the parent's etag governs and the child re-materializes with it.
    pub etag: Option<String>,
    pub properties: serde_json::Map<String, Value>,
}

impl MappedChild {
    /// Parse one entry of a mapper's `children` array.
    ///
    /// Returns `None` for anything missing a usable `name`, `node_type` or
    /// `external_id`. A malformed child is DROPPED, never an error: the parent
    /// message is real content and must still land, and a mapper that emits a
    /// half-built attachment entry should cost that attachment, not the mail.
    /// The caller logs the drop.
    pub fn from_value(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let str_at = |k: &str| {
            obj.get(k)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        };
        // A child's name becomes a path SEGMENT, so a `/` in it would silently
        // graft the node somewhere else in the tree — a provider-supplied
        // filename is untrusted input, and "../x" is a real attachment name.
        let name = sanitize_child_name(str_at("name")?);
        Some(Self {
            name,
            node_type: str_at("node_type")?.to_string(),
            external_id: str_at("external_id")?.to_string(),
            etag: str_at("etag").map(|s| s.to_string()),
            properties: obj
                .get("properties")
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default(),
        })
    }
}

/// Reduce a provider-supplied child name to one safe path segment.
fn sanitize_child_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Built-in Rust default mapping (§4.2): folder → `raisin:Folder`, everything
/// else → `raisin:Node` with title + a `meta` object carrying mime/size/urls and
/// provider passthrough. Zero function invocations.
///
/// Divergence from the frozen contract, which names `raisin:Asset`: that type
/// carries a `title` and a file-shaped schema that a generic link-only mount has
/// nothing to put in, so `raisin:Node` is the permissive carrier for the
/// no-mapper default. A mount that wants `raisin:Asset` supplies a custom
/// `mapping_function` — as the mail mappers do for attachments, which ARE
/// `raisin:Asset` and are metadata-only until `get_content` runs (`file` is
/// optional as of raisin:Asset v3 precisely so that state is representable).
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
