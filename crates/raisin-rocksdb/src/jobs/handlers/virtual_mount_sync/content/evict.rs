// SPDX-License-Identifier: BSL-1.1

//! Dropping cached file bytes once nothing needs them any more.
//!
//! A mount's files live at the provider; our copy is a CACHE. It is filled when
//! something needs the bytes — to extract text, render a thumbnail, serve a
//! preview — and this is what empties it again, so indexing a drive does not
//! mean mirroring it.
//!
//! # What survives
//!
//! Only the bytes go. The node stays, and so does everything derived from it:
//! `__extracted_text`, the embedding, the thumbnail. Those are kilobytes and are
//! the point of having fetched the file at all; the source is megabytes and can
//! be asked for again.
//!
//! `file_type` and `file_size` also stay. They describe the REMOTE file, so they
//! are still true — and they are what the processing rules match on, so dropping
//! them would stop the node ever being re-planned.
//!
//! # Sibling of, not the same as, `ephemeral`
//!
//! [`super::super::ephemeral::cleanup_expired`] deletes NODES past a TTL (the
//! mailbox pattern). This deletes BYTES and keeps the node. A mount routinely
//! wants one without the other, which is why they are separate policies —
//! conflating them would mean enabling a cache also deleted the tenant's
//! documents.

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{NodeRepository, Storage};

use super::super::materializer::{PUSHED_CONTENT_KEY, PUSHED_STATE_PROP};
use super::super::{VirtualMountSyncHandler, SYNC_ACTOR};
use super::CONTENT_CACHED_AT_PROP;

impl VirtualMountSyncHandler {
    /// Drop cached bytes whose `__content_cached_at + ttl` has passed.
    ///
    /// Returns how many nodes were emptied. Best-effort throughout: a node that
    /// cannot be re-read or re-written is left for the next run, because the
    /// cost of missing one is a stale copy on disk while the cost of failing the
    /// sync over it is the whole mount stopping.
    pub(in crate::jobs::handlers::virtual_mount_sync) async fn evict_expired_content(
        &self,
        ctx: &super::super::ctx::SyncCtx<'_>,
        candidates: Vec<(String, bool)>,
    ) -> usize {
        let mut emptied = 0usize;

        for (node_id, has_etag) in candidates {
            if !has_etag {
                // THE LOOP GUARD. Without a provider etag this node's identity
                // falls back to its local hash / storage key / size — the very
                // things eviction changes — so dropping the bytes would re-open
                // the extraction gate, re-download, re-extract and evict again,
                // forever, at the provider's expense.
                tracing::debug!(
                    node_id = %node_id,
                    "Keeping cached content: the node carries no __etag, so evicting \
                     it would change its identity and re-trigger extraction"
                );
                continue;
            }

            match self.evict_one(ctx, &node_id).await {
                Ok(true) => emptied += 1,
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(
                        node_id = %node_id, error = %e,
                        "Could not drop cached content; leaving it for the next run"
                    );
                }
            }
        }

        if emptied > 0 {
            tracing::info!(emptied, "content TTL dropped cached mount files");
        }
        emptied
    }

    /// Empty one node's cache. `false` when it had nothing to drop.
    async fn evict_one(
        &self,
        ctx: &super::super::ctx::SyncCtx<'_>,
        node_id: &str,
    ) -> raisin_error::Result<bool> {
        let scope = &ctx.scope;
        let Some(mut node) = self
            .storage
            .nodes()
            .get(
                raisin_storage::StorageScope::new(
                    &scope.tenant,
                    &scope.repo,
                    &scope.branch,
                    &scope.workspace,
                ),
                node_id,
                None,
            )
            .await?
        else {
            return Ok(false);
        };

        let Some(storage_key) = strip_cached_content(&mut node.properties) else {
            return Ok(false);
        };

        // Node first, object second. The reverse order would leave a node
        // pointing at bytes that no longer exist if the commit then failed —
        // a broken preview — whereas this order can at worst orphan an object,
        // which costs disk and nothing else.
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(&scope.tenant, &scope.repo)?;
        tx.set_branch(&scope.branch)?;
        tx.set_actor(SYNC_ACTOR)?;
        // Written as the sync actor so neither the capture filter nor the
        // reconcile walk reads this as a user edit — an eviction must never be
        // pushed back to the provider as a deletion.
        tx.set_auth_context(AuthContext::system_as(SYNC_ACTOR))?;
        tx.set_message("virtual mount: drop cached content")?;
        tx.upsert_node(&scope.workspace, &node).await?;
        tx.commit().await?;

        if let Some(delete) = &self.binary_delete {
            if let Err(e) = delete(storage_key.clone()).await {
                // Orphaned object. Logged rather than raised: the cache entry is
                // already gone from the node, so the node is correct and only
                // the disk is not.
                tracing::warn!(
                    node_id = %node_id, storage_key = %storage_key, error = %e,
                    "Dropped a cached file from its node but could not delete the object"
                );
            }
        }

        Ok(true)
    }
}

/// Select the nodes whose cached bytes have expired.
///
/// `has_etag` is REQUIRED and is the loop guard, not an optimisation. A mounted
/// asset is identified by `mount|{external_id}|{etag}`; when the provider
/// supplies no etag the fingerprint falls back to the LOCAL hash / storage key /
/// size, all of which eviction changes. Evicting such a node would therefore
/// re-open the extraction gate, re-download the file, re-extract it and evict it
/// again — forever, at the provider's expense. All four bundled adapters supply
/// an etag; some individual nodes (a moved one, an IMAP send receipt) do not.
pub fn expired_content<'a, I>(nodes: I, ttl_seconds: u64, now_secs: i64) -> Vec<(String, bool)>
where
    I: Iterator<Item = (&'a str, Option<i64>, bool)>,
{
    nodes
        .filter_map(|(id, cached_secs, has_etag)| {
            let cached = cached_secs?;
            if cached + ttl_seconds as i64 > now_secs {
                return None;
            }
            Some((id.to_string(), has_etag))
        })
        .collect()
}

/// Strip the cache from a node's properties, leaving everything derived from it.
///
/// Returns the storage key that should now be deleted, if there was one.
pub fn strip_cached_content(
    properties: &mut std::collections::HashMap<String, PropertyValue>,
) -> Option<String> {
    let storage_key = raisin_models::nodes::asset_storage_key(properties);

    properties.remove("file");
    // Describes bytes we no longer hold.
    properties.remove("content_hash");
    properties.remove(CONTENT_CACHED_AT_PROP);

    // `__pushed_state.__content` records the key we are deleting. Left behind it
    // points at an object that no longer exists, and a later re-fetch mints a
    // NEW key — which the write path reads as content that diverges from what
    // was pushed, re-nominating the node for upload on a mount that accepts
    // content. The node would upload its own file back to the provider.
    if let Some(PropertyValue::Object(pushed)) = properties.get_mut(PUSHED_STATE_PROP) {
        pushed.remove(PUSHED_CONTENT_KEY);
    }

    storage_key
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn only_bytes_past_the_ttl_are_selected() {
        let rows = vec![
            ("fresh", Some(1_000i64), true),
            ("stale", Some(100i64), true),
            ("never-cached", None, true),
        ];
        let got = expired_content(rows.into_iter(), 300, 1_000);
        assert_eq!(got, vec![("stale".to_string(), true)]);
    }

    /// The loop guard. A node with no provider etag falls back to a fingerprint
    /// derived from the very bytes we would be deleting, so evicting it would
    /// re-download and re-extract it forever.
    #[test]
    fn a_node_without_an_etag_is_reported_so_the_caller_can_skip_it() {
        let rows = vec![("no-etag", Some(100i64), false)];
        let got = expired_content(rows.into_iter(), 300, 1_000);
        assert_eq!(
            got,
            vec![("no-etag".to_string(), false)],
            "selection reports it; the caller refuses it, and the reason must be \
             visible rather than silently filtered here"
        );
    }

    #[test]
    fn stripping_keeps_everything_derived_from_the_bytes() {
        let mut props: HashMap<String, PropertyValue> = HashMap::new();
        props.insert(
            "file_type".to_string(),
            PropertyValue::String("application/pdf".into()),
        );
        props.insert("file_size".to_string(), PropertyValue::Integer(1234));
        props.insert(
            "__extracted_text".to_string(),
            PropertyValue::String("the text".into()),
        );
        props.insert(
            "__extract_status".to_string(),
            PropertyValue::String("ok".into()),
        );
        props.insert(
            "content_hash".to_string(),
            PropertyValue::String("abc".into()),
        );
        props.insert(
            CONTENT_CACHED_AT_PROP.to_string(),
            PropertyValue::String("2026-09-01T00:00:00Z".into()),
        );

        strip_cached_content(&mut props);

        // Gone: the cache and what describes it.
        assert!(!props.contains_key("file"));
        assert!(!props.contains_key("content_hash"));
        assert!(!props.contains_key(CONTENT_CACHED_AT_PROP));

        // Kept: the point of having fetched the file at all.
        assert!(props.contains_key("__extracted_text"));
        assert_eq!(
            props.get("__extract_status"),
            Some(&PropertyValue::String("ok".into()))
        );
        // Kept: still true of the REMOTE file, and what the rules match on.
        assert!(props.contains_key("file_type"));
        assert!(props.contains_key("file_size"));
    }
}
