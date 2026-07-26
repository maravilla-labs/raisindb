//! Shared parser for `ORDERED_CHILDREN` keys.
//!
//! Key layout (see [`crate::keys::ordered_child_key_versioned`]):
//!
//! ```text
//! {tenant}\0{repo}\0{branch}\0{workspace}\0ordered\0{parent_id}\0{order_label}\0{~revision}\0{child_id}
//! └────────────────────── prefix (ends with \0) ──────────────────────┘
//! ```
//!
//! The `~revision` component is a 16-byte descending-encoded HLC that **may
//! contain null bytes** (notably when the counter is 0), so the tail of the key
//! can never be split on `\0`. Everything after the prefix is parsed by
//! position instead: `order_label` up to the next `\0`, then exactly 16 bytes of
//! revision, then `\0`, then the rest is `child_id`.
//!
//! Before this module the same parse was hand-rolled in four places with three
//! slightly different sets of edge-case handling. Everything now funnels here.

use raisin_hlc::HLC;

/// Number of bytes in a descending-encoded HLC revision.
const REVISION_LEN: usize = 16;

/// A parsed `ORDERED_CHILDREN` key. Borrows from the key buffer.
#[derive(Debug)]
pub(in crate::repositories::nodes) struct ParsedOrderedKey<'a> {
    /// Full order label, including the `::{HLC}` suffix. Use
    /// [`crate::fractional_index::extract_fractional`] for the ordering part.
    pub order_label: &'a str,
    /// Raw descending-encoded revision bytes; decode with [`Self::revision`].
    pub revision_bytes: &'a [u8],
    pub child_id: &'a str,
}

impl ParsedOrderedKey<'_> {
    /// Decode the entry's revision.
    pub(in crate::repositories::nodes) fn revision(&self) -> Option<HLC> {
        crate::keys::decode_descending_revision(self.revision_bytes).ok()
    }
}

/// Parse an `ORDERED_CHILDREN` key whose prefix length is already known.
///
/// Returns `None` for malformed keys, for keys that do not belong to `prefix`,
/// and for the per-parent metadata entries (the `LAST` child cache), which share
/// the parent prefix but are not children. Callers should treat `None` as
/// "skip this entry".
pub(in crate::repositories::nodes) fn parse_ordered_child_key<'a>(
    key: &'a [u8],
    prefix: &[u8],
) -> Option<ParsedOrderedKey<'a>> {
    if !key.starts_with(prefix) {
        return None;
    }
    let suffix = &key[prefix.len()..];

    let label_end = suffix.iter().position(|&b| b == 0)?;
    let order_label = std::str::from_utf8(&suffix[..label_end]).ok()?;

    if is_metadata_label(order_label) {
        return None;
    }

    // order_label \0 {16-byte revision} \0 child_id
    let revision_start = label_end + 1;
    let revision_end = revision_start + REVISION_LEN;
    let child_start = revision_end + 1;
    if suffix.len() < child_start {
        return None;
    }

    Some(ParsedOrderedKey {
        order_label,
        revision_bytes: &suffix[revision_start..revision_end],
        child_id: std::str::from_utf8(&suffix[child_start..]).ok()?,
    })
}

/// True for the per-parent metadata entries that share a parent's prefix.
///
/// [`crate::keys::last_child_metadata_key`] writes `\u{FFFF}META\0LAST` in the
/// `order_label` position. `\u{FFFF}` is not representable as a single byte, so
/// it arrives as multi-byte UTF-8 / a replacement char depending on the path —
/// hence the marker-substring checks rather than an exact match.
fn is_metadata_label(order_label: &str) -> bool {
    order_label.starts_with('\u{FFFF}')
        || order_label.starts_with('\u{FFFD}')
        || order_label.contains("META")
        || order_label.contains("LAST")
        || order_label.contains("FIRST")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys;

    fn prefix() -> Vec<u8> {
        keys::ordered_children_prefix("t", "r", "main", "ws", "parent-1")
    }

    #[test]
    fn parses_a_well_formed_key() {
        let rev = HLC::new(12345, 7);
        let key = keys::ordered_child_key_versioned(
            "t",
            "r",
            "main",
            "ws",
            "parent-1",
            "80::00000000000f4241",
            &rev,
            "child-9",
        );

        let parsed = parse_ordered_child_key(&key, &prefix()).expect("should parse");
        assert_eq!(parsed.order_label, "80::00000000000f4241");
        assert_eq!(parsed.child_id, "child-9");
        assert_eq!(parsed.revision(), Some(rev));
    }

    /// A counter of 0 puts null bytes inside the descending revision encoding —
    /// the exact case that makes splitting the key on `\0` wrong.
    #[test]
    fn parses_keys_whose_revision_contains_null_bytes() {
        let rev = HLC::new(u64::MAX, 0);
        let key = keys::ordered_child_key_versioned(
            "t",
            "r",
            "main",
            "ws",
            "parent-1",
            "80",
            &rev,
            "child-nulls",
        );

        let encoded = &key[prefix().len()..];
        assert!(
            encoded.contains(&0u8),
            "test is only meaningful if the revision encoding contains a null byte"
        );

        let parsed = parse_ordered_child_key(&key, &prefix()).expect("should parse");
        assert_eq!(parsed.order_label, "80");
        assert_eq!(parsed.child_id, "child-nulls");
        assert_eq!(parsed.revision(), Some(rev));
    }

    #[test]
    fn rejects_the_last_child_metadata_entry() {
        let key = keys::last_child_metadata_key("t", "r", "main", "ws", "parent-1");
        assert!(parse_ordered_child_key(&key, &prefix()).is_none());
    }

    #[test]
    fn rejects_keys_from_a_different_parent() {
        let rev = HLC::new(1, 1);
        let key = keys::ordered_child_key_versioned(
            "t",
            "r",
            "main",
            "ws",
            "other-parent",
            "80",
            &rev,
            "child-1",
        );
        assert!(parse_ordered_child_key(&key, &prefix()).is_none());
    }

    #[test]
    fn rejects_truncated_keys() {
        let rev = HLC::new(1, 1);
        let full = keys::ordered_child_key_versioned(
            "t", "r", "main", "ws", "parent-1", "80", &rev, "child-1",
        );
        // Chop into the revision bytes: label terminator present, tail too short.
        let truncated = &full[..full.len() - 10];
        assert!(parse_ordered_child_key(truncated, &prefix()).is_none());

        // No label terminator at all.
        let bare = prefix();
        assert!(parse_ordered_child_key(&bare, &prefix()).is_none());
    }
}
