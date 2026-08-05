//! Reading typed values off a node's property map, and building the property
//! map a materialized node is written with.

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

pub(super) fn req_string(node: &Node, key: &str) -> Result<String, String> {
    opt_string(node, key).ok_or_else(|| format!("missing required property `{key}`"))
}

pub(super) fn opt_string(node: &Node, key: &str) -> Option<String> {
    match node.properties.get(key)? {
        PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

pub(super) fn opt_bool(node: &Node, key: &str) -> Option<bool> {
    match node.properties.get(key)? {
        PropertyValue::Boolean(b) => Some(*b),
        _ => None,
    }
}

pub(crate) fn parse_object<T: for<'de> Deserialize<'de>>(node: &Node, key: &str) -> Option<T> {
    let pv = node.properties.get(key)?;
    let json = serde_json::to_value(pv).ok()?;
    serde_json::from_value(json).ok()
}

/// Like [`parse_object`], but distinguishing ABSENT from PRESENT-BUT-CORRUPT.
///
/// `Ok(None)` means the property is not there, which for a fresh mount is
/// correct and a default is the right answer. `Err` means it IS there and did
/// not parse — and defaulting on that is how a mount silently loses everything
/// it knows about itself.
///
/// For `state` specifically the default is catastrophic and invisible:
/// `last_sync_token: None` sends the mount back to a full walk,
/// `backfill_complete: false` disables reconcile deletes, and `state_seq: 0`
/// against a stored seq that has advanced makes
/// [`crate::jobs::handlers::virtual_mount_sync::persist_mount_state`] refuse
/// EVERY subsequent write — permanently. The mount then materializes nodes it
/// can never record, on every run, while reporting itself healthy.
pub(crate) fn parse_object_checked<T: for<'de> Deserialize<'de>>(
    node: &Node,
    key: &str,
) -> Result<Option<T>, String> {
    let Some(pv) = node.properties.get(key) else {
        return Ok(None);
    };
    let json = serde_json::to_value(pv)
        .map_err(|e| format!("property `{key}` is not serializable: {e}"))?;
    serde_json::from_value(json)
        .map(Some)
        .map_err(|e| format!("property `{key}` is present but could not be parsed: {e}"))
}

/// The raw JSON value of a property, or [`Value::Null`] when absent or
/// unserializable. Used to forward author-supplied config objects verbatim.
pub(super) fn raw_value(node: &Node, key: &str) -> Value {
    node.properties
        .get(key)
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Null)
}

/// Convert a mapped property map plus reserved virtual metadata into node
/// properties (`PropertyValue` map). Reserved keys always overwrite whatever a
/// mapping produced.
pub fn build_properties(
    mapped: &serde_json::Map<String, Value>,
    virt: &crate::jobs::handlers::virtual_mount_sync::materializer::VirtualMeta,
) -> HashMap<String, PropertyValue> {
    let mut out: HashMap<String, PropertyValue> = HashMap::new();
    for (k, v) in mapped {
        if k.starts_with("__") {
            continue; // engine owns reserved props
        }
        if let Ok(pv) = serde_json::from_value::<PropertyValue>(v.clone()) {
            out.insert(k.clone(), pv);
        }
    }
    out.insert("__virtual".to_string(), PropertyValue::Boolean(true));
    out.insert(
        "__mount_id".to_string(),
        PropertyValue::String(virt.mount_id.clone()),
    );
    out.insert(
        "__external_id".to_string(),
        PropertyValue::String(virt.external_id.clone()),
    );
    if let Some(etag) = &virt.etag {
        out.insert("__etag".to_string(), PropertyValue::String(etag.clone()));
    }
    out.insert(
        "__synced_at".to_string(),
        PropertyValue::String(virt.synced_at.clone()),
    );
    out
}
