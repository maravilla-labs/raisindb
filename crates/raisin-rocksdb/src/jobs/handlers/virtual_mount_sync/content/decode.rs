//! Decoding an adapter's `get_content` answer, and the child-id arithmetic that
//! tells the adapter which object to fetch.
//!
//! Split from [`super`] so the pure, cheap-to-test half is not sitting inside
//! the half that needs storage, a credential and a binary store.

use base64::Engine as _;
use raisin_error::{Error, Result};
use raisin_models::nodes::Node;
use serde_json::Value;

use super::super::config::CHILD_ID_SEP;
use super::super::materializer::node_str_prop;

/// Ceiling on a single fetched object, so one adapter answering with a 900 MB
/// string cannot take the process down. The whole payload crosses the QuickJS
/// boundary as a JS string before the engine ever sees it, so this is a
/// second-line guard, not the only one.
const MAX_CONTENT_BYTES: usize = 64 * 1024 * 1024;

/// Split a namespaced child external id back into (own id, parent id).
///
/// A top-level item has no parent, so it addresses itself and the second half is
/// `None` — which is exactly what an adapter whose items are self-addressing
/// wants to see.
pub(crate) fn split_external_id(external_id: &str) -> (String, Option<String>) {
    match external_id.split_once(CHILD_ID_SEP) {
        Some((parent, own)) => (own.to_string(), Some(parent.to_string())),
        None => (external_id.to_string(), None),
    }
}

/// Decode an adapter's `get_content` answer into bytes plus a mime type.
///
/// Two shapes, and the distinction is not cosmetic: `content` is a STRING and a
/// JS string cannot hold arbitrary bytes — a PDF round-tripped through one comes
/// back corrupted with no error anywhere. So any adapter returning binary must
/// send `content_base64`, and `content` is reserved for text (a message body,
/// an exported doc). Preferring base64 when both are present means an adapter
/// that adds it later is immediately correct.
pub(crate) fn decode_content(result: &Value, node: &Node) -> Result<(Vec<u8>, String)> {
    let mime = result
        .get("mime_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| node_str_prop(node, "file_type"))
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let bytes = if let Some(b64) = result.get("content_base64").and_then(|v| v.as_str()) {
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| Error::Validation(format!("get_content returned invalid base64: {e}")))?
    } else if let Some(text) = result.get("content").and_then(|v| v.as_str()) {
        text.as_bytes().to_vec()
    } else {
        return Err(Error::Validation(
            "get_content returned neither `content_base64` nor `content`".to_string(),
        ));
    };

    // A ZERO-LENGTH PAYLOAD IS NOT A SUCCESSFUL FETCH.
    //
    // Storing it as one latches the failure: the node ends up with `file` set
    // and a content hash of nothing, so it looks fetched, and no later run will
    // ever try again — only an explicit `?force=true` clears it. The triggers
    // are all transient (a `body: null` from Graph, an empty 200, a truncated
    // response), which is exactly the case where retrying would have worked.
    //
    // An adapter with a genuinely empty object to represent must say so, rather
    // than leaving "the provider gave us nothing" and "the file is empty"
    // spelled the same way.
    if bytes.is_empty() && result.get("empty").and_then(|v| v.as_bool()) != Some(true) {
        return Err(Error::Validation(
            "get_content returned an empty payload; treating it as a failed fetch rather than \
             an empty file (set `empty: true` if the object really is empty)"
                .to_string(),
        ));
    }

    if bytes.len() > MAX_CONTENT_BYTES {
        return Err(Error::Validation(format!(
            "get_content returned {} bytes, over the {MAX_CONTENT_BYTES}-byte ceiling",
            bytes.len()
        )));
    }
    Ok((bytes, mime))
}

pub(crate) fn content_hash(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::handlers::virtual_mount_sync::config::child_external_id;
    use raisin_models::nodes::properties::PropertyValue;
    use serde_json::json;

    fn asset(name: &str) -> Node {
        Node {
            id: "n1".to_string(),
            node_type: "raisin:Asset".to_string(),
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// The namespacing is reversible, which is the whole reason an adapter can
    /// address the attachment at all: Graph needs BOTH the message id and the
    /// attachment id, and only the parent half carries the message.
    #[test]
    fn a_child_external_id_round_trips_to_item_plus_parent() {
        let ext = child_external_id("AAMkAGI2", "att-7");
        assert_eq!(
            split_external_id(&ext),
            ("att-7".to_string(), Some("AAMkAGI2".to_string()))
        );
        // A top-level item addresses itself.
        assert_eq!(
            split_external_id("AAMkAGI2"),
            ("AAMkAGI2".to_string(), None)
        );
    }

    /// Binary content MUST come back base64. Reading a PDF out of a JS string
    /// silently corrupts it, so `content` is text-only by contract and base64
    /// wins whenever both are present.
    #[test]
    fn base64_content_is_preferred_and_decoded() {
        let pdf = vec![0x25u8, 0x50, 0x44, 0x46, 0x00, 0xFF, 0xFE];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&pdf);
        let result = json!({
            "content_base64": encoded,
            "content": "this text half must lose",
            "mime_type": "application/pdf",
        });
        let (bytes, mime) = decode_content(&result, &asset("q.pdf")).unwrap();
        assert_eq!(bytes, pdf);
        assert_eq!(mime, "application/pdf");
    }

    /// An empty payload is a FAILED fetch, not an empty file.
    ///
    /// Accepting it latches: the node ends up with `file` set and a hash of
    /// nothing, so it looks fetched and no later run tries again — only an
    /// explicit `?force=true` clears it. Every trigger is transient (a
    /// `body: null`, an empty 200, a truncated response), which is exactly when
    /// retrying would have worked.
    #[test]
    fn an_empty_payload_is_a_failed_fetch_not_an_empty_file() {
        for shape in [json!({ "content": "" }), json!({ "content_base64": "" })] {
            let err = decode_content(&shape, &asset("empty.bin")).unwrap_err();
            assert!(
                err.to_string().contains("empty payload"),
                "{shape} should be rejected, got: {err}"
            );
        }

        // An adapter with a genuinely empty object to represent says so, so
        // "the provider gave us nothing" and "the file is empty" stay
        // distinguishable.
        let (bytes, _) =
            decode_content(&json!({ "content": "", "empty": true }), &asset("empty.bin")).unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn text_content_is_accepted_and_the_node_supplies_a_missing_mime() {
        let mut node = asset("note.txt");
        node.properties.insert(
            "file_type".to_string(),
            PropertyValue::String("text/plain".to_string()),
        );
        let (bytes, mime) = decode_content(&json!({ "content": "hello" }), &node).unwrap();
        assert_eq!(bytes, b"hello");
        assert_eq!(mime, "text/plain");
    }

    #[test]
    fn an_answer_with_no_content_is_an_error_not_an_empty_file() {
        // Writing a zero-byte Resource would make `file != null` mean "fetched"
        // forever, and no retry could ever recover the attachment.
        let err = decode_content(&json!({ "mime_type": "application/pdf" }), &asset("q.pdf"));
        assert!(err.is_err());
    }
}
