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
        };
        let result = invoker
            .invoke(&scope, &adapter_path, input)
            .await
            .map_err(|e| Error::storage(format!("get_content failed: {e}")))?;

        let (bytes, mime) = decode_content(&result, &node)?;
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
        // NOT `__etag`. The etag is the provider's change token for the ITEM and
        // is what the sync's skip-write compares; overwriting it here would make
        // the next sync believe the attachment metadata had changed (or, worse,
        // that it had not). `__synced_at` is refreshed because this write IS a
        // sync write and the TTL cleanup measures staleness by it.
        node.properties.insert(
            "__synced_at".to_string(),
            PropertyValue::String(Utc::now().to_rfc3339()),
        );

        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(tenant, repo)?;
        tx.set_branch(branch)?;
        tx.set_actor(SYNC_ACTOR)?;
        // The auth context, not the raw actor, is what stamps `updated_by`.
        tx.set_auth_context(AuthContext::system_as(SYNC_ACTOR))?;
        tx.set_message("virtual mount: fetch attachment content")?;
        tx.upsert_node(workspace, &node).await?;
        tx.commit().await?;

        Ok(ContentFetch::Stored {
            bytes: size,
            storage_key: stored.key,
        })
    }
}
