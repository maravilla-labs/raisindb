//! Key builders and fixed-tail parsers for `cf::LOCALIZED_NAME_INDEX`.
//!
//! - forward `{t}\0{r}\0{b}\0{ws}\0lname\0{locale}\0{parent_id}\0{name}\0{node_id}\0{~rev}`
//!   -> empty (a claim) | `T` (this node gave the claim up);
//! - reverse `{t}\0{r}\0{b}\0{ws}\0lname_of\0{node_id}\0{locale}\0{~rev}`
//!   -> JSON `{parent_id, name}` | `T` (no name in that locale from then on).
//!
//! The node id sits in the forward key BEFORE the revision, so each node's
//! claims on one `(locale, parent, name)` are their own version group: one
//! node's tombstone never masks another node's claim, and history GC's
//! "newest per group" keeps every node's state. Every text segment is
//! null-free (names are validated by the selector); the revision is the fixed
//! 16-byte trailer and is never found by splitting on `\0`.

use crate::keys::KeyBuilder;
use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};

/// `{t}\0{r}\0{b}\0{ws}\0` of a localized-name key.
#[derive(Debug, Clone, Copy)]
pub struct NameScope<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub workspace: &'a str,
}

impl<'a> NameScope<'a> {
    pub fn new(tenant_id: &'a str, repo_id: &'a str, branch: &'a str, workspace: &'a str) -> Self {
        Self {
            tenant_id,
            repo_id,
            branch,
            workspace,
        }
    }

    fn base(&self) -> KeyBuilder {
        KeyBuilder::new()
            .push(self.tenant_id)
            .push(self.repo_id)
            .push(self.branch)
            .push(self.workspace)
    }
}

/// The parent key of a root child (the ORDERED_CHILDREN convention).
pub(crate) const ROOT_PARENT: &str = "/";

/// `…\0lname\0{locale}\0{parent_id}\0{name}\0` — every claim on one segment.
pub(crate) fn forward_prefix(
    scope: NameScope<'_>,
    locale: &str,
    parent_id: &str,
    name: &str,
) -> Vec<u8> {
    scope
        .base()
        .push("lname")
        .push(locale)
        .push(parent_id)
        .push(name)
        .build_prefix()
}

/// The forward key of `node_id`'s claim at `revision`.
pub(crate) fn forward_key(
    scope: NameScope<'_>,
    locale: &str,
    parent_id: &str,
    name: &str,
    node_id: &str,
    revision: &HLC,
) -> Vec<u8> {
    scope
        .base()
        .push("lname")
        .push(locale)
        .push(parent_id)
        .push(name)
        .push(node_id)
        .push_revision(revision)
        .build()
}

/// `…\0lname_of\0{node_id}\0` — every locale's reverse rows of one node.
pub(crate) fn reverse_node_prefix(scope: NameScope<'_>, node_id: &str) -> Vec<u8> {
    scope.base().push("lname_of").push(node_id).build_prefix()
}

/// `…\0lname_of\0{node_id}\0{locale}\0` — one locale's reverse rows.
pub(crate) fn reverse_locale_prefix(scope: NameScope<'_>, node_id: &str, locale: &str) -> Vec<u8> {
    scope
        .base()
        .push("lname_of")
        .push(node_id)
        .push(locale)
        .build_prefix()
}

/// The reverse key of `node_id` in `locale` at `revision`.
pub(crate) fn reverse_key(
    scope: NameScope<'_>,
    node_id: &str,
    locale: &str,
    revision: &HLC,
) -> Vec<u8> {
    scope
        .base()
        .push("lname_of")
        .push(node_id)
        .push(locale)
        .push_revision(revision)
        .build()
}

/// What a live reverse row says: the node's segment in one locale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub parent_id: String,
    pub name: String,
}

pub(crate) fn encode_segment(segment: &Segment) -> Vec<u8> {
    serde_json::to_vec(segment).unwrap_or_default()
}

/// `None` for a tombstone (or an unreadable value, which no writer produces).
pub(crate) fn decode_segment(value: &[u8]) -> Option<Segment> {
    if crate::keys::is_tombstone_value(value) {
        return None;
    }
    serde_json::from_slice(value).ok()
}

/// A parsed forward key: `(workspace, locale, parent_id, name, node_id, revision)`.
pub(crate) struct ForwardKey {
    pub(crate) workspace: String,
    pub(crate) locale: String,
    pub(crate) parent_id: String,
    pub(crate) name: String,
    pub(crate) node_id: String,
    pub(crate) revision: HLC,
}

/// The text segments before the 16-byte revision trailer of `key`
/// (`{text}\0{~rev}`), split on `\0` — safe because only the trailer may
/// contain null bytes.
fn text_segments(key: &[u8]) -> Option<(Vec<&str>, HLC)> {
    if key.len() < 17 || key[key.len() - 17] != 0 {
        return None;
    }
    let revision = HLC::decode_descending(&key[key.len() - 16..]).ok()?;
    let text = std::str::from_utf8(&key[..key.len() - 17]).ok()?;
    Some((text.split('\0').collect(), revision))
}

/// Parse a forward key (`None` for any other shape).
pub(crate) fn parse_forward_key(key: &[u8]) -> Option<ForwardKey> {
    let (parts, revision) = text_segments(key)?;
    if parts.len() != 9 || parts[4] != "lname" {
        return None;
    }
    Some(ForwardKey {
        workspace: parts[3].to_string(),
        locale: parts[5].to_string(),
        parent_id: parts[6].to_string(),
        name: parts[7].to_string(),
        node_id: parts[8].to_string(),
        revision,
    })
}

/// Parse a reverse key into `(locale, revision)` (`None` for any other shape).
pub(crate) fn parse_reverse_key(key: &[u8]) -> Option<(String, HLC)> {
    let (parts, revision) = text_segments(key)?;
    if parts.len() != 7 || parts[4] != "lname_of" {
        return None;
    }
    Some((parts[6].to_string(), revision))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_and_reverse_keys_round_trip_with_a_null_in_the_revision() {
        let scope = NameScope::new("t", "r", "main", "ws");
        // counter 0 encodes to 0xFF.. bytes, timestamp u64::MAX to 0x00..
        let revision = HLC::new(u64::MAX, 0);
        let key = forward_key(scope, "fr", "p1", "chaise", "n1", &revision);
        let parsed = parse_forward_key(&key).unwrap();
        assert_eq!(parsed.locale, "fr");
        assert_eq!(parsed.parent_id, "p1");
        assert_eq!(parsed.name, "chaise");
        assert_eq!(parsed.node_id, "n1");
        assert_eq!(parsed.revision, revision);
        assert!(key.starts_with(&forward_prefix(scope, "fr", "p1", "chaise")));

        let key = reverse_key(scope, "n1", "fr-CA", &revision);
        assert_eq!(
            parse_reverse_key(&key),
            Some(("fr-CA".to_string(), revision))
        );
        assert!(key.starts_with(&reverse_locale_prefix(scope, "n1", "fr-CA")));
        assert!(parse_forward_key(&key).is_none());
    }
}
