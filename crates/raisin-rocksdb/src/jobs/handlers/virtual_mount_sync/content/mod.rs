//! On-demand content fetch: the one caller of an adapter's `get_content`.
//!
//! Sync writes attachment METADATA only — a `raisin:Asset` child per attachment
//! with its name, mime type and size, and no bytes. Downloading every
//! attachment of every synced message during the sync hot loop would multiply a
//! mailbox import by whole documents, and most attachments are never opened.
//!
//! So the bytes are fetched HERE, once, when something asks for them, and are
//! stored in the binary store exactly as any other asset's are. `file == null`
//! on a mount-owned `raisin:Asset` means precisely "not fetched yet"; after this
//! runs it holds a `Resource` pointing at a real stored object.
//!
//! This is also the first exercise of `get_content` in the whole engine — three
//! adapters implement it and nothing has ever called it.

use chrono::Utc;
use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::Resource;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use serde_json::{json, Value};
use std::collections::HashMap;

use super::materializer::node_str_prop;
use super::{
    adapter::build_input, resolve::CtxParts, MountConfig, VirtualMountSyncHandler, SYNC_ACTOR,
};

mod decode;
mod evict;
mod fetch_url;

pub(super) use evict::expired_content;

/// When a mount-owned asset's bytes were last fetched, RFC 3339.
///
/// The clock the CONTENT ttl measures, deliberately not `__synced_at`: that one
/// times the node, and a node outlives many copies of its bytes. Presence also
/// means "this node currently holds cached bytes", which is what the sweep
/// enumerates on.
pub(crate) const CONTENT_CACHED_AT_PROP: &str = "__content_cached_at";

/// How long a cached copy survives when the mount names no `content_ttl_seconds`.
///
/// Thirty minutes: long enough that the jobs which follow one another — extract,
/// thumbnail, embed — all still find the bytes, and that someone browsing does
/// not pay a provider round-trip per click. Short enough that a large drive does
/// not accumulate.
///
/// A DEFAULT rather than "keep forever", because the failure modes are not
/// symmetric: expiring too eagerly costs a re-download, while never expiring
/// mirrors somebody's OneDrive onto this disk.
pub(crate) const DEFAULT_CONTENT_TTL_SECONDS: u64 = 30 * 60;

pub(super) use decode::{content_hash, decode_content, split_external_id};

/// Which node to fetch content for, and where to read the configuration that
/// says how.
///
/// A struct rather than six positional `&str`s because two of them are BRANCHES
/// that must not be the same value, and a positional call site cannot show that:
/// `branch` addresses the MATERIALIZED node (a mount's `target_branch`) while
/// `config_branch` is where the mount and integration nodes live — the same
/// split every sync run makes. Passing the target branch for both silently
/// resolves no mount at all.
#[derive(Debug, Clone, Copy)]
pub struct ContentTarget<'a> {
    pub tenant: &'a str,
    pub repo: &'a str,
    /// Branch the mount / integration configuration is read from.
    pub config_branch: &'a str,
    /// Branch the materialized node lives on.
    pub branch: &'a str,
    pub workspace: &'a str,
    pub node_id: &'a str,
}

/// What a fetch did. `AlreadyPresent` is an ordinary answer, not an error: two
/// viewers opening the same attachment must not both download it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentFetch {
    Stored { bytes: usize, storage_key: String },
    AlreadyPresent,
}

impl VirtualMountSyncHandler {
    /// Fetch a mount-owned asset's bytes, resolving the mount's config branch.
    ///
    /// The one entry point a caller outside this module should use. [`Self::fetch_content`]
    /// takes a [`ContentTarget`] carrying TWO branches that must not be the same
    /// value — `branch` addresses the materialized node, `config_branch` is
    /// where the mount and integration nodes live — and passing one for the
    /// other resolves no mount at all, silently. Every caller was resolving that
    /// itself, and one of them (the integrations HTTP layer) hardcodes `main`,
    /// which is wrong on a repo whose default branch is not `main`.
    ///
    /// Here it is resolved the way the sync engine resolves it, once.
    pub async fn fetch_node_content(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
    ) -> Result<ContentFetch> {
        let config_branch = super::check::config_branch(&self.storage, tenant, repo).await;
        self.fetch_content(
            ContentTarget {
                tenant,
                repo,
                config_branch: &config_branch,
                branch,
                workspace,
                node_id,
            },
            false,
        )
        .await
    }

    /// Fetch the bytes behind one mount-owned `raisin:Asset` and store them.
    ///
    /// `force` re-fetches a node that already carries a `file`; the default is to
    /// answer [`ContentFetch::AlreadyPresent`], so two viewers opening the same
    /// attachment do not both download it.
    pub async fn fetch_content(
        &self,
        target: ContentTarget<'_>,
        force: bool,
    ) -> Result<ContentFetch> {
        let ContentTarget {
            tenant,
            repo,
            config_branch,
            branch,
            workspace,
            node_id,
        } = target;
        let node = self
            .read_asset_node(&target)
            .await?
            .ok_or_else(|| Error::NotFound(format!("node {node_id} not found")))?;

        if !force && node.properties.contains_key("file") {
            return Ok(ContentFetch::AlreadyPresent);
        }

        let mount_id = node_str_prop(&node, "__mount_id").ok_or_else(|| {
            Error::Validation(format!(
                "node {node_id} is not mount-owned (no __mount_id); nothing to fetch"
            ))
        })?;
        let external_id = node_str_prop(&node, "__external_id")
            .ok_or_else(|| Error::Validation(format!("node {node_id} carries no __external_id")))?;

        let (item_id, parent_item_id) = split_external_id(&external_id);

        let svc = self.system_service(tenant, repo, config_branch);
        let mount_node = svc
            .get(&mount_id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("mount {mount_id} not found")))?;
        let mount = MountConfig::from_node(&mount_node)
            .map_err(|e| Error::Validation(format!("invalid mount {mount_id}: {e}")))?;

        let CtxParts {
            adapter_path,
            credential,
            mount_snapshot,
            ..
        } = self
            .build_ctx_parts(&svc, tenant, repo, config_branch, &mount)
            .await?;

        let invoker = self.invoker.clone().ok_or_else(|| {
            Error::Validation("no function executor wired; cannot invoke an adapter".to_string())
        })?;

        // `parent_item_id` is sent alongside `item_id` because a mail attachment
        // is not addressable on its own: Graph's route is
        // `/messages/{message}/attachments/{attachment}`. An adapter whose items
        // ARE self-addressing (Drive) ignores it.
        let params = json!({
            "item_id": item_id,
            "parent_item_id": parent_item_id,
            "mime_type": node_str_prop(&node, "file_type"),
        });
        let input = build_input("get_content", params, &credential, &mount_snapshot);
        let scope = super::MountScope {
            tenant: tenant.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            mount_id: mount_id.clone(),
            mount_path: mount.mount_path.clone(),
            force_rewrite: false,
            watched_fields: Vec::new(),
            // Moot while `watched_fields` is empty, but explicitly off: a
            // content fetch never merges anything.
            read_local_wins: false,
            // Empty for the same reason: this scope exists only to invoke
            // `get_content`, and nothing on that path stages a node.
            command_node_types: Vec::new(),
        };
        let result = invoker
            .invoke(&scope, &adapter_path, input)
            .await
            .map_err(|e| Error::storage(format!("get_content failed: {e}")))?;

        // Three answer shapes, and the third is the one an adapter reaches for
        // when it CANNOT carry the bytes: a provider that hands out a
        // pre-authenticated, short-lived download URL. The engine fetches that
        // itself — in Rust, where bytes survive — behind the operator's egress
        // policy. See `fetch_url` for why neither half of that is optional.
        let (bytes, mime) = match result.get("fetch_url").and_then(|v| v.as_str()) {
            Some(url) => {
                let declared = result
                    .get("mime_type")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| node_str_prop(&node, "file_type"));
                fetch_url::fetch_content_url(url, declared).await?
            }
            None => decode_content(&result, &node)?,
        };
        self.store_content(&target, node, bytes, mime).await
    }

    /// Read the node at latest committed state.
    async fn read_asset_node(&self, target: &ContentTarget<'_>) -> Result<Option<Node>> {
        use raisin_storage::{NodeRepository, Storage};
        let scope = raisin_storage::BranchScope::new(target.tenant, target.repo, target.branch)
            .with_workspace(target.workspace);
        self.storage.nodes().get(scope, target.node_id, None).await
    }

    /// Put the bytes in the binary store and stamp the `Resource` onto the node.
    async fn store_content(
        &self,
        target: &ContentTarget<'_>,
        mut node: Node,
        bytes: Vec<u8>,
        mime: String,
    ) -> Result<ContentFetch> {
        let (tenant, repo, branch, workspace) =
            (target.tenant, target.repo, target.branch, target.workspace);
        let binary_store = self.binary_store.clone().ok_or_else(|| {
            Error::Validation(
                "no binary storage callback configured; attachment content cannot be stored"
                    .to_string(),
            )
        })?;

        let size = bytes.len();
        let hash = content_hash(&bytes);
        let extension = node
            .name
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_string())
            .filter(|e| !e.is_empty() && e.len() <= 16);
        let stored = binary_store(
            bytes,
            Some(mime.clone()),
            extension,
            Some(node.name.clone()),
            Some(tenant.to_string()),
        )
        .await?;

        let mut resource_metadata = HashMap::new();
        resource_metadata.insert(
            "storage_key".to_string(),
            PropertyValue::String(stored.key.clone()),
        );
        let resource = Resource {
            uuid: nanoid::nanoid!(),
            name: stored.name.clone(),
            size: Some(stored.size),
            mime_type: Some(mime.clone()),
            url: Some(stored.url.clone()),
            metadata: Some(resource_metadata),
            is_loaded: Some(true),
            is_external: Some(false),
            created_at: stored.created_at.into(),
            updated_at: stored.updated_at.into(),
        };

        node.properties
            .insert("file".to_string(), PropertyValue::Resource(resource));
        node.properties
            .insert("file_type".to_string(), PropertyValue::String(mime));
        node.properties
            .insert("file_size".to_string(), PropertyValue::Integer(stored.size));
        node.properties
            .insert("content_hash".to_string(), PropertyValue::String(hash));

        // Record these bytes as the PROVIDER's state, not just as ours.
        //
        // The write path treats "this node has content that was never pushed"
        // as diverged, which is right for a file a user replaced and wrong for
        // one the engine just downloaded FROM the provider — without this, the
        // first drain of a mirror mount uploads every fetched file straight
        // back where it came from. Unlike baselining a watched FIELD, this
        // asserts nothing the engine did not verify: the bytes are the
        // provider's, because the provider is where they were read.
        let identity = serde_json::json!({
            "storage_key": stored.key.clone(),
            "size": stored.size,
        });
        let mut pushed = super::materializer::pushed_state_of(&node).unwrap_or_default();
        pushed.insert(
            super::materializer::PUSHED_CONTENT_KEY.to_string(),
            identity,
        );
        if let Ok(value) =
            serde_json::from_value::<PropertyValue>(serde_json::Value::Object(pushed))
        {
            node.properties
                .insert(super::materializer::PUSHED_STATE_PROP.to_string(), value);
        }
        // NOT `__etag`. The etag is the provider's change token for the ITEM and
        // is what the sync's skip-write compares; overwriting it here would make
        // the next sync believe the attachment metadata had changed (or, worse,
        // that it had not). `__synced_at` is refreshed because this write IS a
        // sync write and the TTL cleanup measures staleness by it.
        node.properties.insert(
            "__synced_at".to_string(),
            PropertyValue::String(Utc::now().to_rfc3339()),
        );

        // When this cache was filled — the clock the content TTL measures.
        //
        // Separate from `__synced_at`, which the NODE ttl uses: the two expire
        // different things and a node routinely outlives many copies of its
        // bytes. Refreshed on every fetch, so a re-read extends the lease.
        node.properties.insert(
            CONTENT_CACHED_AT_PROP.to_string(),
            PropertyValue::String(Utc::now().to_rfc3339()),
        );

        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(tenant, repo)?;
        tx.set_branch(branch)?;
        tx.set_actor(SYNC_ACTOR)?;
        // The auth context, not the raw actor, is what stamps `updated_by`.
        tx.set_auth_context(AuthContext::system_as(SYNC_ACTOR))?;
        // ENGINE BOOKKEEPING, not a content change. This write moves the
        // content CACHE — `file`, `content_hash`, `__content_cached_at`,
        // `__synced_at` — and never `__extracted_text` or any property a
        // vector or a trigger is built from. Marking it keeps the cache cycle
        // from re-enqueueing an embedding job per asset per TTL period, which
        // is work that could only ever conclude the text had not changed.
        tx.set_bookkeeping(true)?;
        tx.set_message("virtual mount: fetch attachment content")?;
        tx.upsert_node(workspace, &node).await?;
        tx.commit().await?;

        Ok(ContentFetch::Stored {
            bytes: size,
            storage_key: stored.key,
        })
    }
}
