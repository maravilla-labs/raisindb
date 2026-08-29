//! The `state_only` write view of a node: what the mount watches, and what it
//! last pushed.
//!
//! Kept beside the index rather than inside it because BOTH the index load and
//! the staging path derive it, and they must never disagree about what "the
//! current value" means — a writer and a reader with two notions of that is how
//! a converge check turns into an infinite push loop.

use raisin_models::nodes::Node;
use serde_json::{Map, Value};

/// Reserved property recording the watched fields as they were last PUSHED.
///
/// Engine-owned (`config/props.rs` drops every `__` key a mapper produces), and
/// required rather than nice-to-have: a provider's etag does not necessarily
/// move when a `state_only` field does — Graph's `isRead` changes
/// `@odata.etag`, IMAP's `\Seen` does not move `MODSEQ` usefully — so the etag
/// alone cannot answer "has this flag already been pushed?". Without it a
/// `state_only` mount either pushes the same flag on every run forever or drops
/// the change entirely.
pub const PUSHED_STATE_PROP: &str = "__pushed_state";

/// Key under which a node's file-content identity is recorded inside
/// [`PUSHED_STATE_PROP`].
///
/// Reserved, and namespaced away from any real property: it is not a field an
/// adapter or a mapper ever sees. It exists because a mount that carries BYTES
/// has a second kind of divergence, and the watched-field machinery cannot see
/// it — replacing a file's contents changes no property a `mutable_fields` list
/// would sensibly contain.
pub const PUSHED_CONTENT_KEY: &str = "__content";

/// The `state_only` write view of one node: the watched fields as they stand
/// now, and the values last pushed to the provider.
///
/// Only the watched fields are kept, never the whole property map — a mail
/// mount's index holds every message in the mailbox, and carrying bodies in it
/// would trade the per-item workspace scan this index exists to remove for a
/// memory blowup of the same size.
#[derive(Debug, Clone, Default)]
pub struct WriteView {
    /// Current values of the mount's watched fields, absent keys omitted.
    pub watched: Map<String, Value>,
    /// [`PUSHED_STATE_PROP`] as stored. `None` means this node has never been
    /// stamped — NOT that nothing was ever pushed.
    pub pushed: Option<Map<String, Value>>,
    /// Identity of the node's FILE BYTES, when it carries any: the binary
    /// store's key and the size.
    ///
    /// The key is minted per stored object (`nanoid`), so replacing a file's
    /// contents always produces a new one — which makes this a reliable "the
    /// bytes changed" signal without hashing anything. Size rides along as a
    /// cheap second opinion.
    ///
    /// Deliberately NOT a member of `watched`: `mutable_fields` means "node
    /// property names the engine hands the mapper as `fields`", and a storage
    /// key is not a provider property. Putting it there would oblige every
    /// adapter and every custom mapper to know one field name it must silently
    /// ignore, and the one that forgot would send it to the provider.
    pub content: Option<Value>,
}

impl WriteView {
    /// Whether this node predates writeback on this mount.
    ///
    /// A node with no [`PUSHED_STATE_PROP`] carries no evidence either way, and
    /// both answers cost something: treating it as diverged pushes the values it
    /// currently holds back at the provider, once per node, which on an existing
    /// mailbox is a burst of writes (bounded by `max_items_per_sync` per run);
    /// treating it as converged silently drops the first local edit on every
    /// node that existed before writeback was turned on.
    ///
    /// The drain takes the first: it pushes. Baselining locally — recording the
    /// node's OWN values as "what the provider has" — reads like a third,
    /// cheaper option and is not one, because the remote values are not in scope
    /// anywhere in the drain and no adapter `get` operation exists. It asserts
    /// something the engine did not verify, and when it is wrong the edit is
    /// lost with no error. See `write::push::push_one`.
    ///
    /// Kept as a predicate because the distinction is still real for anything
    /// reporting on a mount; the drain itself no longer branches on it.
    pub fn is_unseeded(&self) -> bool {
        self.pushed.is_none()
    }

    /// Whether any watched field differs from what was last pushed.
    ///
    /// A field present locally and absent from the pushed map counts as
    /// diverged: that is exactly a first edit of a field the provider never
    /// reported.
    pub fn diverges(&self, fields: &[String]) -> bool {
        let pushed = self.pushed.as_ref();
        fields
            .iter()
            .any(|f| self.watched.get(f) != pushed.and_then(|p| p.get(f)))
    }

    /// The subset of `fields` that actually differ from what was last pushed.
    ///
    /// A push must carry ONLY these — never the mount's whole allow-list. The
    /// distinction is not cosmetic: some provider fields have side effects on
    /// mere PRESENCE in an update. Microsoft Graph resends meeting invitations
    /// to every attendee whenever `attendees` appears in a PATCH, changed or
    /// not — so a mount that allows attendee edits used to spam every attendee
    /// each time someone fixed a typo in the title.
    /// Whether the node's BYTES differ from those last pushed.
    ///
    /// Only meaningful on a mount whose adapter declared `accepts_content`;
    /// the caller gates on that, because for every other mount the file
    /// resource is metadata the provider owns and re-pushing it would be an
    /// echo, not an edit.
    ///
    /// A node with content and no recorded content counts as diverged — the
    /// first push of a file the provider has never been given, which is exactly
    /// the create case and, on a mount that gained `accepts_content` later, the
    /// backlog it should work through.
    pub fn content_diverges(&self) -> bool {
        let Some(current) = self.content.as_ref() else {
            return false;
        };
        match self.pushed.as_ref().and_then(|p| p.get(PUSHED_CONTENT_KEY)) {
            Some(pushed) => pushed != current,
            None => true,
        }
    }

    pub fn diverged_fields(&self, fields: &[String]) -> Vec<String> {
        let pushed = self.pushed.as_ref();
        fields
            .iter()
            .filter(|f| self.watched.get(*f) != pushed.and_then(|p| p.get(*f)))
            .cloned()
            .collect()
    }
}

/// The watched-field view of one node, or `None` when the mount watches
/// nothing. Shared by index load and by the staging path, so the two can never
/// disagree about what "current value" means.
pub fn write_view_of(node: &Node, watched_fields: &[String]) -> Option<Box<WriteView>> {
    let content = content_identity(node);
    // A mirror may legitimately watch no fields at all and still have something
    // to push: a file node's bytes. Returning `None` here would make that mount
    // nominate nothing, forever, with no error.
    if watched_fields.is_empty() && content.is_none() {
        return None;
    }
    let mut watched = Map::new();
    for field in watched_fields {
        if let Some(pv) = node.properties.get(field) {
            if let Ok(v) = serde_json::to_value(pv) {
                watched.insert(field.clone(), v);
            }
        }
    }
    Some(Box::new(WriteView {
        watched,
        pushed: pushed_state_of(node),
        content,
    }))
}

/// The identity of a node's file bytes, if it carries any.
///
/// Read from the `file` Resource the read path stamps (and an upload writes) —
/// the same property `write::content` sends to the adapter, so the thing
/// compared here is the thing pushed.
pub fn content_identity(node: &Node) -> Option<Value> {
    let raisin_models::nodes::properties::PropertyValue::Resource(resource) =
        node.properties.get("file")?
    else {
        return None;
    };
    let key = match resource.metadata.as_ref()?.get("storage_key")? {
        raisin_models::nodes::properties::PropertyValue::String(s) if !s.is_empty() => s.clone(),
        _ => return None,
    };
    Some(serde_json::json!({ "storage_key": key, "size": resource.size }))
}

/// [`PUSHED_STATE_PROP`] exactly as stored on a node, independent of what the
/// mount currently watches.
pub fn pushed_state_of(node: &Node) -> Option<Map<String, Value>> {
    node.properties
        .get(PUSHED_STATE_PROP)
        .and_then(|pv| serde_json::to_value(pv).ok())
        .and_then(|v| match v {
            Value::Object(map) => Some(map),
            _ => None,
        })
}

/// The `local_wins` pre-merge for one incoming upsert: the mapper output with
/// pending local edits overlaid, and the baseline to reseed alongside it.
///
/// `None` when nothing is pending — the caller must then take the ordinary
/// reseed path untouched, which is what keeps `remote_wins` behaviour
/// instruction-identical on every mount that has not opted in.
///
/// `live` is the node as it stands NOW, not the run-start index view: a user
/// edit landing after index load (the exact mid-run race the Hue postmortem
/// documents) is invisible to the index, and reverting an edit because it was
/// recent is precisely the failure `local_wins` exists to close.
///
/// Three rules, one per map:
///
/// * a pending field whose INCOMING value already equals the local one is
///   dropped from the pending set first — the remote already holds the edit,
///   and reseeding its baseline from the item is the correct convergence
///   rather than a pointless no-op push next drain. This is the same "a remote
///   change converges on arrival" invariant the ordinary path enforces, not an
///   exception to it.
/// * the merged properties carry the LOCAL value of every remaining pending
///   field — including its ABSENCE: a local delete of the field must not be
///   resurrected by the incoming value.
/// * the reseeded baseline starts from the incoming item (non-diverged watched
///   fields must still converge) and then keeps the OLD stored entry for
///   exactly the pending fields — absent stays absent, because "the provider
///   never reported this" and "the provider reported null" are different
///   answers and conflating them hides the first edit of an unreported field
///   from [`WriteView::diverges`].
///
/// The net effect: the node keeps the user's values, the baseline keeps the
/// evidence that they are un-pushed, and the next drain nominates them exactly
/// as if the remote item had never arrived — while `__etag` (stamped by the
/// caller from the incoming item) names the remote version we knowingly
/// overrode, so that push normally succeeds without ever reaching the 409
/// force path.
pub(super) fn preserve_pending_edits(
    incoming: &Map<String, Value>,
    live: &WriteView,
    watched_fields: &[String],
) -> Option<(Map<String, Value>, Map<String, Value>)> {
    let pending: Vec<String> = live
        .diverged_fields(watched_fields)
        .into_iter()
        .filter(|f| incoming.get(f) != live.watched.get(f))
        .collect();
    if pending.is_empty() {
        return None;
    }
    // `watched_subset` rather than a local loop, for the same reason this
    // module exists at all: two definitions of "the watched subset" is how a
    // reseed and a converge check start disagreeing. `pending` is non-empty
    // here, so `watched_fields` is non-empty and the subset is `Some`.
    let mut baseline = super::stamp::watched_subset(incoming, watched_fields).unwrap_or_default();
    let mut merged = incoming.clone();
    for field in &pending {
        match live.watched.get(field) {
            Some(local) => {
                merged.insert(field.clone(), local.clone());
            }
            None => {
                merged.remove(field);
            }
        }
        match live.pushed.as_ref().and_then(|p| p.get(field)) {
            Some(old) => {
                baseline.insert(field.clone(), old.clone());
            }
            None => {
                baseline.remove(field);
            }
        }
    }
    Some((merged, baseline))
}

/// The stored baseline to carry on an index entry, given the write view already
/// derived from the same node.
///
/// `None` when the view holds it, so a watching mount stores one copy rather
/// than two — the map is small but a mail mount's index holds every message in
/// the mailbox. What this exists for is the OTHER case: a mount whose
/// `watched_fields` are empty (any mode but `state_only`) derives no write view
/// at all, and without a carried copy an upsert would rebuild the node's
/// property map with no `__pushed_state` in it and destroy the baseline.
pub fn carried_pushed_state(
    node: &Node,
    write_view: &Option<Box<WriteView>>,
) -> Option<Map<String, Value>> {
    if write_view.is_some() {
        return None;
    }
    pushed_state_of(node)
}
