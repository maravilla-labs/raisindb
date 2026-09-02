//! Mapping between external items and node shapes, in both directions.
//!
//! A `mapping_function` is a single function node that dispatches on
//! `input.operation`, so the inbound translation (`to_node`) and the outbound
//! one (`to_external`) have one author and one file. Splitting them — reverse
//! mapping inside the adapter, forward mapping in the mapper — would express one
//! relationship twice and let a custom mapper silently disagree with the adapter
//! it is pointed at. See `docs/reference/virtual-node-adapters.md` §6.0.

use super::*;

/// A mapper's reverse translation of one node into a provider-shaped payload.
#[derive(Debug, Clone)]
pub struct ToExternal {
    /// Provider-shaped body for the adapter's `create` / `update` / `submit`.
    /// When the engine asked for a field allow-list this holds ONLY those keys.
    pub payload: Value,
    /// The remote object to address, when the mapper knows it. `None` leaves the
    /// engine to use the node's `__external_id`.
    pub external_id: Option<String>,
}

/// Outcome of asking a mapper to translate a node outward.
///
/// Three outcomes, not two, and none of them an error: "this mount has no
/// mapper", "this mapper declines this node" and "here is the payload" are all
/// ordinary answers the write path acts on differently, and collapsing the first
/// two into `Err` would make a read-only mount look like a broken one.
#[derive(Debug, Clone)]
pub enum ToExternalOutcome {
    /// The mapper produced a payload.
    Mapped(ToExternal),
    /// The mapper returned `null`: this node is not writable. The write parks
    /// with a stated reason rather than pushing a guess.
    NotWritable,
    /// The mount has no `mapping_function`, so there is nothing to invert — the
    /// built-in Rust [`default_mapping`] is lossy and has no reverse.
    NoMapper,
}

/// Whether a mount's mapper can translate nodes outward at all.
///
/// Probed once per run, alongside the adapter's `capabilities`, because
/// writability belongs to the **mount** — adapter and mapper together. A
/// write-capable adapter behind a read-only custom mapper is not a writable
/// mount, and reporting it as one would give the console a control that
/// silently does nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapperWriteback {
    /// The mapper answered the probe affirmatively.
    Supported,
    /// The mount has no `mapping_function` at all — read-only by construction.
    NoMapper,
    /// The mapper ran but does not implement `to_external`. Every mapper written
    /// before the write path existed lands here: they read only
    /// `input.external_item`, so an unknown operation falls through their
    /// `if (!item …) return null` guard and answers `null`.
    NotImplemented,
    /// The probe itself failed (the mapper threw). Treated as "cannot write",
    /// never as a sync failure.
    ProbeFailed(String),
}

impl MapperWriteback {
    /// Whether the mapper declared `to_external`.
    pub fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }

    /// Operator-facing reason this mapper cannot write, or `None` when it can.
    pub fn reason(&self) -> Option<String> {
        match self {
            Self::Supported => None,
            Self::NoMapper => {
                Some("mount has no mapping_function (the built-in mapping has no reverse)".into())
            }
            Self::NotImplemented => Some("mapper does not implement to_external".into()),
            Self::ProbeFailed(e) => Some(format!("mapper to_external probe failed: {e}")),
        }
    }
}

/// Map one external item to a node, honoring a custom mapping function.
/// Returns `None` when the mapper filters the item out.
pub async fn map_item(
    ctx: &SyncCtx<'_>,
    item: &ExternalItem,
) -> std::result::Result<Option<MappedItem>, AdapterError> {
    // Normalize BEFORE either branch, so the built-in mapping and every custom
    // `mapping_function` see the same item. A provider that omits the mimetype
    // (Graph does, for Office formats) otherwise produces a node no processing
    // rule can match — synced, browsable, and invisible to extraction,
    // thumbnails and embedding, with no error anywhere. See
    // `ExternalItem::ensure_mime_type`.
    let mut normalized = item.clone();
    normalized.ensure_mime_type();
    let item = &normalized;

    let Some(mapper) = &ctx.mount.mapping_function else {
        return Ok(Some(MappedItem {
            node: default_mapping(item),
            children: Vec::new(),
        }));
    };
    // `operation` is additive and every shipped mapper ignores it: a mapper
    // written before dispatch existed reads only `external_item` and `mount`,
    // so an extra key changes nothing. A dispatching mapper gets the `"to_node"`
    // case it switches on. Absent `operation` must keep meaning `to_node` on the
    // JS side for the same reason.
    let input = json!({
        "operation": "to_node",
        "external_item": item,
        "mount": ctx.mount_snapshot,
    });
    let out = ctx.invoker.invoke(&ctx.scope, mapper, input).await?;
    if out.is_null() {
        return Ok(None);
    }
    let node_type = out
        .get("node_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AdapterError::Transient("mapping missing node_type".to_string()))?
        .to_string();
    let name = out
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let properties = out
        .get("properties")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    // Subordinate nodes (mail attachments as `raisin:Asset`). Absent for every
    // mapper written before this existed; a non-array value is ignored rather
    // than rejected, for the same reason a malformed entry is dropped.
    let mut children = Vec::new();
    if let Some(list) = out.get("children").and_then(|v| v.as_array()) {
        for raw in list {
            match MappedChild::from_value(raw) {
                Some(child) => children.push(child),
                None => tracing::warn!(
                    mount_id = %ctx.mount.mount_id,
                    external_id = %item.external_id,
                    "mapper emitted a child without name/node_type/external_id; dropping it"
                ),
            }
        }
    }

    Ok(Some(MappedItem {
        node: MappedNode {
            node_type,
            name,
            properties,
        },
        children,
    }))
}

/// Translate one node outward through the mount's mapper.
///
/// `fields`, when present, is the allow-list a `state_only` mount pushes: the
/// mapper must emit only those keys, so the engine never sends a whole object
/// where a patch was meant.
///
/// `intent` is which operation the payload is destined for — `"create"`,
/// `"update"` or `"submit"`. A mapper that does not read it behaves exactly as
/// before, but the ones that need it genuinely cannot infer it: `fields` is
/// empty for a mirror update as well as for a create, and the difference is not
/// cosmetic. A calendar exception, for instance, is perfectly updatable (it has
/// its own provider id) and impossible to create from nothing (the provider
/// mints one by diverging an occurrence), so the same node must translate
/// outward for one and decline for the other.
///
/// Deliberately NOT routed through [`build_input`]: a mapper is pure and
/// I/O-free and must never receive the connection's credential. It runs inside
/// the write drain, under the mount lease.
pub async fn map_to_external(
    ctx: &SyncCtx<'_>,
    node: &Value,
    fields: Option<&[String]>,
    intent: &str,
) -> std::result::Result<ToExternalOutcome, AdapterError> {
    let Some(mapper) = &ctx.mount.mapping_function else {
        return Ok(ToExternalOutcome::NoMapper);
    };
    let input = json!({
        "operation": "to_external",
        "node": node,
        "mount": ctx.mount_snapshot,
        "fields": fields,
        "intent": intent,
    });
    let out = ctx.invoker.invoke(&ctx.scope, mapper, input).await?;
    // Null is "not writable", exactly as it is "skip this item" for `to_node`.
    // A legacy mapper answers null here too, which is why an un-probed mount can
    // never accidentally push.
    if out.is_null() {
        return Ok(ToExternalOutcome::NotWritable);
    }
    let payload = out
        .get("payload")
        .cloned()
        .ok_or_else(|| AdapterError::Transient("to_external missing payload".to_string()))?;
    let external_id = out
        .get("external_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok(ToExternalOutcome::Mapped(ToExternal {
        payload,
        external_id,
    }))
}

/// Stage one item (map → buffer) at `rel_path`. The batcher decides when the
/// buffered work is actually written.
///
/// An item whose etag already matches what is stored is dropped BEFORE the
/// mapper runs: the skip decision needs only `external_id` + `etag`, both of
/// which the provider supplied, so mapping it first would spend a QuickJS
/// invocation to produce a value that is immediately discarded. On a re-sync of
/// an unchanged mailbox that was the entire cost of the run.
pub async fn stage_item(
    ctx: &SyncCtx<'_>,
    batcher: &mut batch::SyncBatcher<'_>,
    item: &ExternalItem,
    rel_path: &str,
) -> std::result::Result<Vec<String>, AdapterError> {
    if batcher.can_skip_unmapped(item) {
        batcher.note_unmapped_skip();
        // The item was skipped, but its children are still LIVE — and a full
        // walk deletes every mount-owned node it did not report as seen. Return
        // the ones already materialized, or the first unchanged re-sync of a
        // mailbox would delete every attachment node in it and the next changed
        // sync would recreate them: churn forever, on the cheapest path there
        // is. The ids come from the run's index, so this costs no I/O.
        return Ok(batcher.child_external_ids(&item.external_id));
    }
    let Some(mapped) = map_item(ctx, item).await? else {
        return Ok(Vec::new());
    };
    // "I could not enumerate this item's children" is NOT "this item has no
    // children", and the walk cannot tell them apart on its own: both arrive as
    // an empty `children` list, and an empty list means every existing child
    // falls out of `seen` and is pruned.
    //
    // That is a data-destruction path with a routine trigger — one throttled
    // attachment listing on a busy mailbox deleted Asset nodes that had already
    // been imported, permanently, because the parent's etag had not moved and
    // nothing would ever re-enumerate them. An adapter that cannot answer says
    // so with `metadata.children_unknown`, and the live children stay seen.
    let children_unknown = item
        .metadata
        .as_ref()
        .and_then(|m| m.get("children_unknown"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let children = mapped.children;
    batcher.stage_upsert(item, rel_path, mapped.node).await?;

    if children_unknown && children.is_empty() {
        tracing::debug!(
            mount_id = %ctx.mount.mount_id,
            external_id = %item.external_id,
            "adapter could not enumerate children; keeping the ones already materialized"
        );
        return Ok(batcher.child_external_ids(&item.external_id));
    }

    // Children AFTER the parent, in the same buffer, so they apply in that
    // order: `upsert_deep_node` would otherwise auto-create the mail's path as a
    // `raisin:Folder` to hold the attachment, and the real mail node would then
    // land on a path already occupied by a folder of a different type.
    let mut staged = Vec::with_capacity(children.len());
    for child in children {
        let external_id = child_external_id(&item.external_id, &child.external_id);
        let child_item = ExternalItem {
            external_id: external_id.clone(),
            name: child.name.clone(),
            // A child's etag is the PARENT's when the provider has none of its
            // own: attachments of an immutable message never change alone, so
            // inheriting it lets the etag skip-write apply to them too. Without
            // it every child would rewrite on every run.
            etag: child.etag.clone().or_else(|| item.etag.clone()),
            mime_type: None,
            size_bytes: None,
            is_folder: false,
            parent_id: Some(item.external_id.clone()),
            created_at: None,
            modified_at: None,
            web_url: None,
            download_url: None,
            metadata: None,
        };
        batcher
            .stage_upsert(
                &child_item,
                &format!("{}/{}", rel_path.trim_end_matches('/'), child.name),
                MappedNode {
                    node_type: child.node_type,
                    name: Some(child.name),
                    properties: child.properties,
                },
            )
            .await?;
        staged.push(external_id);
    }
    Ok(staged)
}
