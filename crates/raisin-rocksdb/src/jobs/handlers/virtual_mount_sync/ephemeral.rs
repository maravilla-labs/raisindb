//! TTL cleanup for ephemeral mounts (the mailbox / webhook pattern).
//!
//! For mounts with `sync_config.ephemeral: true`, virtual nodes older than
//! `ttl_seconds` are deleted. Runs at the start of every sync of an ephemeral
//! mount, giving auto-cleanup without a dedicated job type.

use raisin_error::Result;

use super::materializer::{MountScope, NodeMaterializer};

/// Delete mount-owned nodes whose `__synced_at + ttl_seconds < now`.
///
/// `now_secs` is unix epoch seconds. Returns the number of nodes deleted.
/// Nodes without a parseable `__synced_at` are left alone (fail-safe).
pub async fn cleanup_expired(
    materializer: &dyn NodeMaterializer,
    scope: &MountScope,
    ttl_seconds: u64,
    now_secs: i64,
) -> Result<usize> {
    let nodes = materializer.list_virtual(scope).await?;
    let mut deleted = 0usize;
    for node in nodes {
        let Some(synced) = node.synced_secs else {
            continue;
        };
        if synced + ttl_seconds as i64 <= now_secs {
            if materializer.delete(scope, &node.external_id).await? {
                deleted += 1;
            }
        }
    }
    if deleted > 0 {
        tracing::info!(
            mount_id = %scope.mount_id,
            deleted,
            "ephemeral cleanup removed expired virtual nodes"
        );
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
