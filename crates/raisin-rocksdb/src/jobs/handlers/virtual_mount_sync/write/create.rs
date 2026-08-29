//! Local CREATE propagation: a node the user authored under a mirror mount
//! becomes an object at the provider.
//!
//! This is the one write direction with no `__external_id` to work from, and
//! that shapes everything here. Every other path starts from `SyncIndex`, which
//! holds only nodes that already carry both `__mount_id` and `__external_id`; a
//! node that has never been synced is by definition absent from it. So the
//! candidates come from a path scan, and the ownership question the index
//! normally answers has to be answered another way — see
//! [`WriteConfig::create_node_types`], which is that answer and is empty by
//! default.
//!
//! **Why creates run before updates.** A create ends in
//! [`SyncBatcher::stage_adopt`], which puts the node into the run's index. The
//! update pass that follows therefore sees it — and correctly decides it has
//! nothing to do, because the create stamped `__pushed_state` from the same
//! values it sent. Running the other way round would leave a just-created node
//! for the *next* run to consider, which is harmless but makes the first drain
//! after a burst of authoring look half-finished.

use serde_json::json;

use super::super::materializer::{estimate_node_bytes, write_view_of};
use super::super::{map_to_external, AdapterError, SyncBatcher, SyncCtx, ToExternalOutcome};

/// What acting on one create candidate did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Created {
    /// `create` was called and the node adopted.
    Adopted,
    /// The mapper declined to translate this node outward. Nothing was sent and
    /// nothing was stamped, so fixing the mapper still creates it next run.
    Skipped,
    /// The adapter returned no usable `external_id`. Treated as a failure
    /// rather than a success precisely because the alternative — adopting the
    /// node with a fabricated id — would make it undeletable and unmatchable
    /// forever, and the next full reconcile would create a SECOND copy at the
    /// provider.
    NoId,
}

/// Create every locally-authored node this mount has opted in, in path order.
///
/// Never fails the run — same contract as the other drains. A provider refusing
/// creates must not stop the mount reading, and what went wrong reaches the
/// operator through `writeback_last_error`.
pub(super) async fn drain_creates(
    ctx: &SyncCtx<'_>,
    batcher: &mut SyncBatcher<'_>,
    state: &mut super::super::MountState,
    stats: &mut super::DrainStats,
    accepts_content: bool,
) {
    let nodes = match ctx.materializer.scan_mount_nodes(&ctx.scope).await {
        Ok(nodes) => nodes,
        Err(e) => {
            tracing::warn!(
                mount_id = %ctx.scope.mount_id,
                error = %e,
                "mount scan failed; nothing was created remotely this run"
            );
            stats.failed += 1;
            state.writeback_last_error = Some(format!("create scan failed: {e}"));
            return;
        }
    };

    // Paths the ENGINE authored to hold mount content, not paths a user
    // authored to be pushed.
    //
    // `upsert_deep_node` auto-creates the ancestor folders a mapped item's path
    // needs, and those folders carry no `__external_id` — which is the whole of
    // what `is_create_candidate` was testing. So a mirror mount with
    // `raisin:Folder` in its `create_node_types` and a hierarchical
    // `path_template` pushed its own scaffolding to the provider, minting a
    // synthetic folder tree next to the real content.
    //
    // The positive evidence is structural: a node that is an ANCESTOR of
    // something this mount owns is scaffolding for that something. Deriving it
    // from the scan we already have costs one pass and no extra read.
    let chain_paths: std::collections::HashSet<String> = nodes
        .iter()
        .filter(|node| node.properties.contains_key("__external_id"))
        .flat_map(|node| super::super::materializer::ancestor_paths(&node.path))
        .collect();

    let mut pending: Vec<raisin_models::nodes::Node> = nodes
        .into_iter()
        .filter(|node| is_create_candidate(ctx, node))
        .filter(|node| {
            if chain_paths.contains(&node.path) {
                tracing::debug!(
                    mount_id = %ctx.scope.mount_id,
                    path = %node.path,
                    "not creating a folder this engine authored to hold mount content"
                );
                return false;
            }
            true
        })
        .collect();
    if pending.is_empty() {
        return;
    }
    // Stable order, and by PATH rather than by id: a create is the one write
    // whose ordering a human can see, and creating a folder's contents in path
    // order is what makes a partially-completed run look sensible rather than
    // arbitrary. It also makes the cap below drop a deterministic tail.
    pending.sort_by(|a, b| a.path.cmp(&b.path));
    let limit = ctx.mount.sync_config.max_items_per_sync.max(1) as usize;
    let truncated = pending.len().saturating_sub(limit);
    pending.truncate(limit);

    for (i, node) in pending.iter().enumerate() {
        // Checked BEFORE starting another provider call, never after: a create
        // already issued cannot be taken back, and one issued without adopting
        // its result is the orphan case this module works hardest to avoid.
        if ctx.out_of_time(chrono::Utc::now().timestamp()) {
            stats.truncated = true;
            stats.pending += pending.len() - i;
            break;
        }
        if i % 5 == 0 && super::super::stop_requested(ctx).await {
            stats.stopped = true;
            stats.pending += pending.len() - i;
            break;
        }
        match create_one(ctx, batcher, node, accepts_content).await {
            Ok(Created::Adopted) => stats.pushed += 1,
            Ok(Created::Skipped) => stats.skipped += 1,
            // Counted as a failure, not a skip: the node may now have a remote
            // counterpart nobody can match. That must be visible in the run's
            // receipt rather than folded into "nothing to do".
            Ok(Created::NoId) => {
                stats.failed += 1;
                if stats.first_error.is_none() {
                    stats.first_error =
                        Some("adapter's create returned no external_id".to_string());
                }
            }
            Err(e) => {
                stats.failed += 1;
                if stats.first_error.is_none() {
                    stats.first_error = Some(e.to_string());
                }
                tracing::warn!(
                    mount_id = %ctx.scope.mount_id,
                    node_id = %node.id,
                    path = %node.path,
                    error = %e,
                    "remote create failed; the node stays local and is retried next run"
                );
                if matches!(e, AdapterError::AuthExpired) {
                    break;
                }
            }
        }
    }
    if truncated > 0 {
        stats.truncated = true;
        stats.pending += truncated;
        tracing::info!(
            mount_id = %ctx.scope.mount_id,
            deferred = truncated,
            "more nodes await remote creation than this run's item cap allows"
        );
    }
}

/// Whether one scanned node is a create candidate for this mount.
///
/// Deliberately strict on both halves. `__external_id` present means the node is
/// already mount-owned, so this is an update, not a create — checking the
/// property rather than index membership matters because the index is a
/// snapshot and a node adopted earlier in this same run is in one and not
/// necessarily the other.
pub(super) fn is_create_candidate(ctx: &SyncCtx<'_>, node: &raisin_models::nodes::Node) -> bool {
    if !ctx.mount.write_config.is_creatable_type(&node.node_type) {
        return false;
    }
    if node.properties.contains_key("__external_id") {
        return false;
    }
    // A derived recurrence occurrence is never pushed in any direction. The
    // projection regenerates it from its series master, so creating one at the
    // provider would mint a duplicate meeting that the next rebuild disowns.
    if crate::jobs::handlers::calendar_expand::path_refusal(&node.path).is_some() {
        return false;
    }
    true
}

/// Reverse-map one node, create it at the provider, and adopt it.
///
/// Failures propagate to the caller unchanged, exactly as the update path does:
/// `auth_expired` must stop the drain, `rate_limited` must requeue, and a
/// `config_error` must mark the mount rather than be counted as one bad item.
pub(super) async fn create_one(
    ctx: &SyncCtx<'_>,
    batcher: &mut SyncBatcher<'_>,
    node: &raisin_models::nodes::Node,
    accepts_content: bool,
) -> Result<Created, AdapterError> {
    // Bytes not here yet? Wait. See `content::content_pending` — creating now
    // means asking a provider whose create IS the byte transfer to make
    // something out of nothing, and its refusal is terminal for the mount.
    if super::content::content_pending(
        node,
        accepts_content,
        &ctx.mount.sync_config.folder_node_types,
    ) {
        tracing::debug!(
            mount_id = %ctx.scope.mount_id,
            node_id = %node.id,
            path = %node.path,
            "node carries no content yet; deferring its remote create until the \
             upload completes"
        );
        return Ok(Created::Skipped);
    }

    let node_bytes = estimate_node_bytes(node);
    let node_json = serde_json::to_value(node)
        .map_err(|e| AdapterError::Transient(format!("node serialize failed: {e}")))?;

    // `fields: None`, unlike an update. A field allow-list is what makes a
    // `state_only` push a PATCH of named properties; a create has nothing to
    // patch and must carry the whole object, or the provider stores an event
    // with no start time and the mount immediately reports it as diverged.
    let payload = match map_to_external(ctx, &node_json, None, "create").await? {
        ToExternalOutcome::Mapped(mapped) => mapped,
        ToExternalOutcome::NotWritable | ToExternalOutcome::NoMapper => {
            tracing::debug!(
                mount_id = %ctx.scope.mount_id,
                node_id = %node.id,
                path = %node.path,
                "mapper declined to translate this node outward; not creating it remotely"
            );
            return Ok(Created::Skipped);
        }
    };

    // Through `call_with_content` rather than `ctx.call` so a node carrying
    // FILE BYTES sends them: metadata alone would create an empty object at the
    // provider that then looks synced forever, because the node and the item
    // agree on everything the engine compares.
    // WHERE the object belongs at the provider.
    //
    // `parent_id` is the mount's own remote root, which is the whole answer for
    // a single-container mount (a calendar) and only half of it for a drive: a
    // file created inside `Gründung` must land in THAT folder, not at the top of
    // the library. So the node's parent folder is resolved to its provider id
    // and sent alongside — the adapter prefers it when the provider has a
    // hierarchy, and ignores it when it does not.
    //
    // `None` when the parent is the mount root itself, or when the parent folder
    // has no provider id yet (it was created locally and not yet pushed). The
    // adapter then falls back to the mount root, which is the old behaviour and
    // the only safe one: filing into a folder we cannot name is not something to
    // guess at.
    let parent_external_id = node
        .path
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .filter(|parent| !parent.is_empty() && *parent != ctx.scope.mount_path)
        .and_then(|parent| batcher.external_id_at(parent));

    let result = super::content::call_with_content(
        ctx,
        "create",
        json!({
            "payload": payload.payload,
            // The mount's own remote root, so an adapter that files new
            // objects into a container (a Drive folder, a named calendar)
            // does not have to re-derive it. Absent on a default-container
            // mount, which is not an error.
            "parent_id": ctx.mount.remote_root,
            "parent_external_id": parent_external_id,
            // The node's location inside the mount, for an adapter that
            // addresses by path rather than by container id.
            "relative_path": node
                .path
                .strip_prefix(&format!("{}/", ctx.scope.mount_path))
                .unwrap_or(&node.name),
        }),
        node,
        accepts_content,
        None,
    )
    .await?;

    // No id, no adoption. An adapter that answers `null`, or an object without
    // `external_id`, has told us nothing we can use to match this node again —
    // and adopting it anyway would be the worst outcome available: the node
    // would look synced, carry an id no provider item has, never converge, and
    // the next reconcile would create a duplicate at the provider because the
    // local node still has no counterpart it can find.
    let external_id = result
        .get("external_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let Some(external_id) = external_id else {
        tracing::warn!(
            mount_id = %ctx.scope.mount_id,
            node_id = %node.id,
            path = %node.path,
            "adapter's create returned no external_id; the node is NOT adopted. \
             If the object was in fact created this leaves an orphan at the provider — \
             which is why an adapter that creates must return the id it created"
        );
        return Ok(Created::NoId);
    };

    let etag = result
        .get("etag")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    // Baseline every watched field, because a create sends the WHOLE node.
    //
    // This is the one push where "what was sent" and "what the node holds" are
    // the same set, so the update path's caution — record only the fields the
    // allow-list actually carried — has nothing to bite on. Baselining here is
    // also not the mistake the seed path was fixed for: that one recorded local
    // values as remote for an object the engine had never written, whereas here
    // the engine just created the remote object FROM these values, so the
    // assertion is one it is entitled to make.
    let mut pushed = write_view_of(node, &ctx.scope.watched_fields)
        .map(|view| view.watched.clone())
        .unwrap_or_default();
    // A create sends the bytes as well as the fields, so the baseline records
    // them — otherwise the very next drain sees content that was never pushed
    // and uploads the same file a second time.
    if accepts_content {
        if let Some(identity) = super::super::materializer::content_identity(node) {
            pushed.insert(
                super::super::materializer::PUSHED_CONTENT_KEY.to_string(),
                identity,
            );
        }
    }

    // Visible to the REST OF THIS RUN before the flush, so a file created after
    // the folder it lives in finds its parent. Creates are processed in path
    // order, which puts a folder ahead of its contents — that ordering is only
    // useful if the folder's provider id is readable by the time the contents
    // are reached.
    batcher.record_adopted(&node.id, &node.path, external_id, etag.clone());

    batcher
        .stage_adopt(&node.id, external_id, etag, Some(pushed), node_bytes)
        .await?;
    Ok(Created::Adopted)
}
