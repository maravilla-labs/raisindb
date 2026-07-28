//! Key encoding for the AUDIT_LOG column family.
//!
//! ```text
//! {tenant}\0{repo}\0{branch}\0{workspace}\0audit\0{node_id}\0{~timestamp_ms}{~seq}
//! ```
//!
//! `audit` is the index tag, matching the `nodes` / `ordered` / `geo` / `uniq`
//! tags used by the other layouts.
//!
//! # Why the suffix is fixed-width and inverted
//!
//! The only read the store must serve is "this node's entries, newest first"
//! (both HTTP audit routes and the WS query resolve to a node id before asking).
//! Inverting the 8-byte big-endian millisecond timestamp — the same trick as
//! `encode_descending_revision`, applied to wall-clock time because `AuditLog`
//! carries a `DateTime<Utc>` and no HLC — makes plain forward iteration return
//! newest-first, so the read path never sorts in memory.
//!
//! `~seq` is a monotonically increasing per-process counter, likewise inverted.
//! It breaks ties within a millisecond, which are entirely possible: `make_log`
//! mints ids at millisecond resolution, so two writes to the same node in the
//! same millisecond would otherwise collide on one key and the second would
//! silently overwrite the first.
//!
//! Both components are appended as ONE fixed-width 16-byte blob rather than two
//! null-separated parts, because bitwise-NOT output can itself contain null
//! bytes — a key that embedded them as separators could not be split back apart
//! unambiguously (the same hazard documented on `extract_revision_from_key`).
//! Nothing needs to parse them back; ordering is the whole job.

use super::KeyBuilder;

/// Index tag for audit-log keys.
const AUDIT_TAG: &str = "audit";

/// Width of the trailing `{~timestamp_ms}{~seq}` blob.
const SUFFIX_LEN: usize = 16;

/// Prefix for every audit entry of one node: `…\0audit\0{node_id}\0`.
///
/// This is exactly the scan range the read path uses.
pub fn audit_node_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push(AUDIT_TAG)
        .push(node_id)
        .build_prefix()
}

/// Full key for one audit entry.
///
/// `timestamp_ms` is the entry's wall-clock time; `seq` disambiguates entries
/// written in the same millisecond. Both are stored inverted so newer entries
/// sort first.
pub fn audit_entry_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    timestamp_ms: i64,
    seq: u64,
) -> Vec<u8> {
    // Bias the signed millisecond timestamp into u64 so pre-epoch values (and
    // any clock skew below 1970) still order correctly, then invert for
    // descending order.
    let biased = (timestamp_ms as u64) ^ (1u64 << 63);
    let mut suffix = Vec::with_capacity(SUFFIX_LEN);
    suffix.extend_from_slice(&(!biased).to_be_bytes());
    suffix.extend_from_slice(&(!seq).to_be_bytes());

    let mut key = audit_node_prefix(tenant_id, repo_id, branch, workspace, node_id);
    key.extend_from_slice(&suffix);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_key_starts_with_node_prefix() {
        let prefix = audit_node_prefix("t", "r", "main", "ws", "node-1");
        let key = audit_entry_key("t", "r", "main", "ws", "node-1", 1_700_000_000_000, 0);
        assert!(key.starts_with(&prefix));
        assert_eq!(key.len(), prefix.len() + SUFFIX_LEN);
    }

    #[test]
    fn newer_entries_sort_first() {
        let older = audit_entry_key("t", "r", "main", "ws", "n", 1_000, 0);
        let newer = audit_entry_key("t", "r", "main", "ws", "n", 2_000, 0);
        assert!(newer < older, "newer entry must sort before older");
    }

    #[test]
    fn same_millisecond_breaks_tie_by_descending_seq() {
        let first = audit_entry_key("t", "r", "main", "ws", "n", 1_000, 1);
        let second = audit_entry_key("t", "r", "main", "ws", "n", 1_000, 2);
        assert_ne!(first, second, "same-ms writes must not collide on one key");
        assert!(second < first, "later write in the same ms must sort first");
    }

    #[test]
    fn pre_epoch_timestamps_still_order() {
        let older = audit_entry_key("t", "r", "main", "ws", "n", -2_000, 0);
        let newer = audit_entry_key("t", "r", "main", "ws", "n", -1_000, 0);
        let modern = audit_entry_key("t", "r", "main", "ws", "n", 1_700_000_000_000, 0);
        assert!(newer < older);
        assert!(modern < newer);
    }

    #[test]
    fn scopes_are_prefix_isolated() {
        let a = audit_node_prefix("t1", "r", "main", "ws", "n");
        let b = audit_node_prefix("t2", "r", "main", "ws", "n");
        let branch_a = audit_node_prefix("t1", "r", "main", "ws", "n");
        let branch_b = audit_node_prefix("t1", "r", "feature", "ws", "n");
        assert_ne!(a, b);
        assert_ne!(branch_a, branch_b);

        // A different node id must not fall inside another node's scan range.
        let other = audit_entry_key("t1", "r", "main", "ws", "n-2", 1_000, 0);
        assert!(!other.starts_with(&a));
    }
}
