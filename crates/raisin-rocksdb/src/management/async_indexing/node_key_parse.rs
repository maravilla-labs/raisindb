//! Parsing a `NODES` key back into its parts, for the scans that walk the
//! column family directly (the `node_path` backfill, the PATH_INDEX repair).

use crate::keys;
use raisin_hlc::HLC;

/// `{branch prefix}{ws}\0nodes\0{id}\0{~rev}` → `(ws, id, rev)`. Any other
/// shape under the branch prefix is not a node version and yields `None`. The
/// revision is the fixed 16-byte trailer (it can contain `\0`).
pub(crate) fn parse_node_key<'k>(
    branch_prefix: &[u8],
    key: &'k [u8],
) -> Option<(&'k str, &'k str, HLC)> {
    let rest = key.strip_prefix(branch_prefix)?;
    if rest.len() < 16 + 1 {
        return None;
    }
    let (head, rev) = rest.split_at(rest.len() - 16);
    let head = head.strip_suffix(b"\0")?;
    let ws_end = head.iter().position(|b| *b == 0)?;
    let (workspace, after_ws) = head.split_at(ws_end);
    let id = after_ws.strip_prefix(b"\0nodes\0")?;
    if id.is_empty() || id.contains(&0) {
        return None;
    }
    let revision = HLC::decode_descending(rev).ok()?;
    Some((
        std::str::from_utf8(workspace).ok()?,
        std::str::from_utf8(id).ok()?,
        revision,
    ))
}

/// `{t}\0{r}\0{b}\0{ws}\0nodes\0` — every version of every node of a
/// workspace, as one prefix.
pub(crate) fn workspace_nodes_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> Vec<u8> {
    keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("nodes")
        .build_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_node_keys_and_nothing_else() {
        let rev = HLC::new(7, 0); // counter 0: the trailer holds \0 bytes
        let key = keys::node_key_versioned("t", "r", "main", "ws", "n1", &rev);
        let prefix = keys::branch_prefix("t", "r", "main");
        assert_eq!(parse_node_key(&prefix, &key), Some(("ws", "n1", rev)));
        assert!(key.starts_with(&workspace_nodes_prefix("t", "r", "main", "ws")));

        let path_key = keys::node_path_key_versioned("t", "r", "main", "ws", "n1", &rev);
        assert_eq!(parse_node_key(&prefix, &path_key), None);
        let other_branch = keys::node_key_versioned("t", "r", "dev", "ws", "n1", &rev);
        assert_eq!(parse_node_key(&prefix, &other_branch), None);
    }
}
