//! Binary blobs that only pruned history still pointed at.
//!
//! A blob key looks like `[{prefix}/…]{yyyy}/{mm}/{dd}/{nanoid}[.{ext}]`
//! (`raisin_binary::FilesystemBinaryStorage::generate_key`; the S3 backend
//! uses the same shape). The `{yyyy}/{mm}/{dd}/{nanoid}` part identifies the
//! blob on its own — the nanoid is 21 random characters — so it is the unit
//! both sides are compared on.
//!
//! The two sides are deliberately asymmetric:
//!
//! * A **candidate** for deletion must be a msgpack string that is EXACTLY a
//!   blob key (decoded with `rmpv`), because its full text is what gets passed
//!   to `BinaryStorage::delete`. A URL or a path merely containing the id is
//!   not a candidate.
//! * A **reference that keeps a blob alive** is any occurrence of the id in any
//!   surviving value, found by a byte search that ignores structure. A URL, a
//!   rendition map, a job context — anything mentioning the id protects it.
//!
//! So a blob is only ever reported when the pruned history named it by key and
//! nothing that survives mentions it in any form.
//!
//! The same rule drives the retroactive sweep ([`super::sweep`]): a blob the
//! store lists is deleted only when its key has the blob shape and no key or
//! value outside the derived indexes mentions its id.

use regex::bytes::Regex as BytesRegex;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

fn exact_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^(?:[A-Za-z0-9_.\-]+/)*(\d{4}/\d{2}/\d{2}/[A-Za-z0-9_\-]{21})(?:\.[A-Za-z0-9]{1,16})?$",
        )
        .expect("valid regex")
    })
}

fn identity_re() -> &'static BytesRegex {
    static RE: OnceLock<BytesRegex> = OnceLock::new();
    RE.get_or_init(|| {
        BytesRegex::new(r"\d{4}/\d{2}/\d{2}/[A-Za-z0-9_\-]{21}").expect("valid regex")
    })
}

/// Blob keys named (exactly) by a msgpack value, as `identity -> full key`.
pub(super) fn exact_keys_in_msgpack(value: &[u8], out: &mut HashMap<String, String>) {
    let mut cursor = std::io::Cursor::new(value);
    let Ok(decoded) = rmpv::decode::read_value(&mut cursor) else {
        return;
    };
    let mut stack = vec![decoded];
    while let Some(v) = stack.pop() {
        match v {
            rmpv::Value::String(s) => {
                if let Some(text) = s.as_str() {
                    if let Some(caps) = exact_key_re().captures(text) {
                        out.insert(caps[1].to_string(), text.to_string());
                    }
                }
            }
            rmpv::Value::Array(items) => stack.extend(items),
            rmpv::Value::Map(entries) => {
                for (k, v) in entries {
                    stack.push(k);
                    stack.push(v);
                }
            }
            _ => {}
        }
    }
}

/// The id of a blob-shaped storage key (`None` for anything else).
pub(super) fn identity_of_key(key: &str) -> Option<&str> {
    exact_key_re()
        .captures(key)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
}

/// Every blob id mentioned anywhere in `value`, in any form.
pub(super) fn mentioned_ids<V>(
    value: &[u8],
    candidates: &HashMap<String, V>,
    out: &mut HashSet<String>,
) {
    if candidates.is_empty() {
        return;
    }
    for m in identity_re().find_iter(value) {
        if let Ok(id) = std::str::from_utf8(m.as_bytes()) {
            if candidates.contains_key(id) {
                out.insert(id.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(v: &rmpv::Value) -> Vec<u8> {
        let mut buf = Vec::new();
        rmpv::encode::write_value(&mut buf, v).unwrap();
        buf
    }

    #[test]
    fn only_exact_keys_become_candidates() {
        let v = rmpv::Value::Array(vec![
            rmpv::Value::from("default/2026/09/29/aWtyQ89hhuq2JkcY9fK_N.rap"),
            rmpv::Value::from("http://host/files/2026/09/29/bWtyQ89hhuq2JkcY9fK_N.jpg"),
            rmpv::Value::from("2026/09/29/cWtyQ89hhuq2JkcY9fK_N"),
            rmpv::Value::from("not a key"),
        ]);
        let mut out = HashMap::new();
        exact_keys_in_msgpack(&pack(&v), &mut out);
        assert_eq!(
            out.get("2026/09/29/aWtyQ89hhuq2JkcY9fK_N")
                .map(String::as_str),
            Some("default/2026/09/29/aWtyQ89hhuq2JkcY9fK_N.rap")
        );
        assert!(out.contains_key("2026/09/29/cWtyQ89hhuq2JkcY9fK_N"));
        // A URL is a reference, never a deletion candidate.
        assert!(!out.contains_key("2026/09/29/bWtyQ89hhuq2JkcY9fK_N"));
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn only_blob_shaped_keys_have_an_identity() {
        assert_eq!(
            identity_of_key("default/2026/09/29/aWtyQ89hhuq2JkcY9fK_N.rap"),
            Some("2026/09/29/aWtyQ89hhuq2JkcY9fK_N")
        );
        assert_eq!(
            identity_of_key("2026/09/29/aWtyQ89hhuq2JkcY9fK_N"),
            Some("2026/09/29/aWtyQ89hhuq2JkcY9fK_N")
        );
        assert_eq!(identity_of_key(".DS_Store"), None);
        assert_eq!(identity_of_key("2026/09/29/short.rap"), None);
        assert_eq!(identity_of_key("default/notes/readme.md"), None);
    }

    #[test]
    fn any_mention_protects_a_candidate() {
        let mut candidates = HashMap::new();
        candidates.insert(
            "2026/09/29/bWtyQ89hhuq2JkcY9fK_N".to_string(),
            "2026/09/29/bWtyQ89hhuq2JkcY9fK_N.jpg".to_string(),
        );
        let raw = b"\xd9\x40http://host/files/2026/09/29/bWtyQ89hhuq2JkcY9fK_N.jpg";
        let mut seen = HashSet::new();
        mentioned_ids(raw, &candidates, &mut seen);
        assert!(seen.contains("2026/09/29/bWtyQ89hhuq2JkcY9fK_N"));
    }
}
