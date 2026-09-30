//! Upper bound on the job result that is written to disk.
//!
//! A job's full result lives in the in-memory registry for the grace window
//! every synchronous caller polls within (`jobs::cleanup::RESULT_GRACE_MINUTES`);
//! the persisted copy only serves history views after that. Results used to
//! be persisted verbatim, and some are large: a trigger evaluation carried the
//! whole changed node, an asset-processing job the entire extracted text
//! (inline base64 images included). On a local dev database that was ~1.7 GB
//! of `job_metadata` for a single day of history — the largest column family
//! by far.
//!
//! Over the bound, the persisted result keeps its small top-level fields (ids,
//! counts, flags, status) and replaces the rest with a marker saying what was
//! dropped and how big it was.

use super::PersistedJobEntry;
use std::borrow::Cow;

/// Largest serialized result persisted verbatim.
pub const MAX_PERSISTED_RESULT_BYTES: usize = 16 * 1024;

/// Top-level fields at most this large survive in a truncated result.
const MAX_KEPT_FIELD_BYTES: usize = 1024;

fn json_len(v: &serde_json::Value) -> usize {
    serde_json::to_vec(v).map(|b| b.len()).unwrap_or(usize::MAX)
}

/// The result to persist for `value`, or `None` when it is within bounds.
pub(super) fn bound_result(value: &serde_json::Value) -> Option<serde_json::Value> {
    let size = json_len(value);
    if size <= MAX_PERSISTED_RESULT_BYTES {
        return None;
    }
    let mut kept = serde_json::Map::new();
    let mut dropped = Vec::new();
    if let serde_json::Value::Object(fields) = value {
        for (k, v) in fields {
            if json_len(v) <= MAX_KEPT_FIELD_BYTES {
                kept.insert(k.clone(), v.clone());
            } else {
                dropped.push(serde_json::Value::String(k.clone()));
            }
        }
    }
    kept.insert(
        "_truncated".to_string(),
        serde_json::json!({
            "original_bytes": size,
            "dropped_fields": dropped,
        }),
    );
    Some(serde_json::Value::Object(kept))
}

/// `entry` with its result bounded for persistence (borrowed when unchanged).
pub(super) fn bounded(entry: &PersistedJobEntry) -> Cow<'_, PersistedJobEntry> {
    match entry.result.as_ref().and_then(bound_result) {
        None => Cow::Borrowed(entry),
        Some(result) => {
            let mut owned = entry.clone();
            owned.result = Some(result);
            Cow::Owned(owned)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn small_results_are_untouched() {
        assert!(bound_result(&json!({"ok": true, "id": "x"})).is_none());
    }

    #[test]
    fn large_fields_are_dropped_and_small_ones_kept() {
        let big = "x".repeat(MAX_PERSISTED_RESULT_BYTES * 2);
        let v = json!({
            "node_id": "n1",
            "success": true,
            "extracted_text": big,
        });
        let out = bound_result(&v).expect("over the bound");
        assert_eq!(out["node_id"], "n1");
        assert_eq!(out["success"], true);
        assert!(out.get("extracted_text").is_none());
        assert_eq!(
            out["_truncated"]["dropped_fields"],
            json!(["extracted_text"])
        );
        assert!(json_len(&out) < MAX_PERSISTED_RESULT_BYTES);
    }

    #[test]
    fn a_large_non_object_becomes_a_marker() {
        let v = json!(vec!["y".repeat(1000); 40]);
        let out = bound_result(&v).expect("over the bound");
        assert!(out["_truncated"]["original_bytes"].as_u64().unwrap() > 16_000);
    }
}
