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
}

/// The watched-field view of one node, or `None` when the mount watches
/// nothing. Shared by index load and by the staging path, so the two can never
/// disagree about what "current value" means.
pub fn write_view_of(node: &Node, watched_fields: &[String]) -> Option<Box<WriteView>> {
    if watched_fields.is_empty() {
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
    }))
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
