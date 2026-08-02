//! TTL cleanup for ephemeral mounts (the mailbox / webhook pattern).
//!
//! For mounts with `sync_config.ephemeral: true`, virtual nodes older than
//! `ttl_seconds` are deleted. Runs at the start of every sync of an ephemeral
//! mount, giving auto-cleanup without a dedicated job type.

use super::batch::SyncBatcher;
use super::AdapterError;

/// Delete mount-owned nodes whose `__synced_at + ttl_seconds < now`.
///
/// `now_secs` is unix epoch seconds. Returns the number of nodes deleted.
/// Nodes without a parseable `__synced_at` are left alone (fail-safe).
///
/// Reads the run's prefetched index rather than re-listing the workspace, and
/// stages the deletes so an expired mailbox is cleared in a handful of
/// transactions instead of one per node.
pub async fn cleanup_expired(
    batcher: &mut SyncBatcher<'_>,
    ttl_seconds: u64,
    now_secs: i64,
) -> std::result::Result<usize, AdapterError> {
    let mut deleted = 0usize;
    for node in batcher.virtual_nodes() {
        let Some(synced) = node.synced_secs else {
            continue;
        };
        if synced + ttl_seconds as i64 <= now_secs {
            batcher.stage_delete(&node.external_id).await?;
            deleted += 1;
        }
    }
    batcher.flush().await?;
    if deleted > 0 {
        tracing::info!(deleted, "ephemeral cleanup removed expired virtual nodes");
    }
    Ok(deleted)
}

/// Parse an ISO 8601 / RFC 3339 timestamp to unix epoch seconds.
fn parse_iso(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rfc3339() {
        let ts = parse_iso("2026-07-10T00:00:00Z").unwrap();
        assert!(ts > 1_700_000_000);
        assert!(parse_iso("not-a-date").is_none());
    }
}
