//! Parsing a translation version key back into its parts.
//!
//! The revision is the fixed 16-byte TAIL (descending HLC, which contains
//! `\0` bytes when the counter is 0), so it is taken from the end and only the
//! head is split on `\0` — the rule `ordering::key_parse` follows for
//! ORDERED_CHILDREN. Workspace, node id, block uuid and locale never contain
//! `\0`.

use raisin_hlc::HLC;

/// One `TRANSLATION_DATA` or `BLOCK_TRANSLATIONS` version key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranslationVersionKey {
    pub workspace: String,
    pub node_id: String,
    /// `Some` for a block overlay.
    pub block_uuid: Option<String>,
    pub locale: String,
    pub revision: HLC,
}

/// Parse `key`, which starts with `branch_prefix` (`{tenant}\0{repo}\0{branch}\0`).
///
/// `None` for anything that is not a translation version of this branch: a
/// key of another branch, a `trans_meta`/`trans_hash` record, or a block orphan
/// marker (it sits in the locale position).
pub(crate) fn parse_version_key(branch_prefix: &[u8], key: &[u8]) -> Option<TranslationVersionKey> {
    let rest = key.strip_prefix(branch_prefix)?;
    if rest.len() < 17 || rest[rest.len() - 17] != 0 {
        return None;
    }
    let (head, tail) = rest.split_at(rest.len() - 17);
    let revision = crate::keys::decode_descending_revision(&tail[1..]).ok()?;
    let parts: Vec<&str> = head
        .split(|b| *b == 0)
        .map(std::str::from_utf8)
        .collect::<Result<_, _>>()
        .ok()?;
    let (workspace, node_id, block_uuid, locale) = match parts.as_slice() {
        [ws, "translations", node, locale] => (*ws, *node, None, *locale),
        [ws, "block_trans", node, block, locale] => (*ws, *node, Some(*block), *locale),
        _ => return None,
    };
    if block_uuid.is_some() && locale == super::keys::BLOCK_ORPHAN_MARKER {
        return None;
    }
    Some(TranslationVersionKey {
        workspace: workspace.to_string(),
        node_id: node_id.to_string(),
        block_uuid: block_uuid.map(str::to_string),
        locale: locale.to_string(),
        revision,
    })
}

#[cfg(test)]
mod tests {
    use super::super::keys;
    use super::*;

    /// Counter 0 puts `\0` bytes inside the revision; parsing must not care.
    #[test]
    fn parses_node_and_block_versions_and_nothing_else() {
        let prefix = b"t\0r\0main\0";
        for rev in [HLC::new(1_705_843_009_213, 0), HLC::new(42, 7)] {
            let key = keys::translation_key("t", "r", "main", "ws", "n1", "de-CH", &rev);
            let parsed = parse_version_key(prefix, &key).unwrap();
            assert_eq!(
                parsed,
                TranslationVersionKey {
                    workspace: "ws".into(),
                    node_id: "n1".into(),
                    block_uuid: None,
                    locale: "de-CH".into(),
                    revision: rev,
                }
            );
            let block = keys::block_translation_key("t", "r", "main", "ws", "n1", "b", "fr", &rev);
            let parsed = parse_version_key(prefix, &block).unwrap();
            assert_eq!(parsed.block_uuid.as_deref(), Some("b"));
            assert_eq!(parsed.locale, "fr");
            assert_eq!(parsed.revision, rev);

            let orphan = keys::block_orphan_key("t", "r", "main", "ws", "n1", "b", &rev);
            assert_eq!(parse_version_key(prefix, &orphan), None);
            let meta = keys::translation_meta_key("t", "r", "main", "ws", "n1", "fr", &rev);
            assert_eq!(parse_version_key(prefix, &meta), None);
            assert_eq!(parse_version_key(b"t\0r\0other\0", &key), None);
        }
    }
}
