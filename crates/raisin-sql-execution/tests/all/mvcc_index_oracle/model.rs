//! The reference model: an in-memory tree driven ONLY by the generated op log.
//!
//! It never reads `cf::NODES`, `NODE_PATH`, `ORDERED_CHILDREN` or any
//! repository API. Parents are tracked by ID (never by the `Node.parent` NAME,
//! which is ambiguous), and sibling order is an explicit `Vec` per parent, so
//! editorial order is checked against an independent truth rather than against
//! the index it is meant to verify.
//!
//! Timestamps are modelled as the op sequence number that stamped them: the
//! model cannot know wall-clock micros, but it knows which op wrote last, and
//! the driver records a wall-clock instant BETWEEN ops so ranges can be built.

use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const ROOT: &str = "/";

/// A node's French overlay, as the op log wrote it.
#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    Title(String),
    Hidden,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MNode {
    pub id: String,
    pub name: String,
    /// Parent node ID (`ROOT` for a top-level node).
    pub parent: String,
    pub node_type: String,
    pub props: HashMap<String, PropertyValue>,
    /// Op sequence that stamped `created_at`.
    pub created: u64,
    /// Op sequence that last stamped `updated_at`.
    pub updated: u64,
    pub overlay: Option<Overlay>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    pub nodes: BTreeMap<String, MNode>,
    /// Explicit editorial order, keyed by parent ID.
    pub children: HashMap<String, Vec<String>>,
}

impl Tree {
    pub fn get(&self, id: &str) -> Option<&MNode> {
        self.nodes.get(id)
    }

    pub fn kids(&self, parent: &str) -> &[String] {
        self.children.get(parent).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn path(&self, id: &str) -> String {
        let mut segs = Vec::new();
        let mut cur = id;
        while cur != ROOT {
            let n = &self.nodes[cur];
            segs.push(n.name.as_str());
            cur = &n.parent;
        }
        segs.reverse();
        format!("/{}", segs.join("/"))
    }

    pub fn by_path(&self, path: &str) -> Option<&MNode> {
        self.nodes.values().find(|n| self.path(&n.id) == path)
    }

    /// Live ids, sorted (the generator's selector domain).
    pub fn ids(&self) -> Vec<String> {
        self.nodes.keys().cloned().collect()
    }

    pub fn name_taken(&self, parent: &str, name: &str) -> bool {
        self.kids(parent).iter().any(|c| self.nodes[c].name == name)
    }

    /// `id` and every descendant, pre-order (parents before children).
    pub fn subtree(&self, id: &str) -> Vec<String> {
        let mut pre = Vec::new();
        self.preorder(id, &mut pre);
        pre
    }

    fn preorder(&self, id: &str, out: &mut Vec<String>) {
        out.push(id.to_string());
        for c in self.kids(id) {
            self.preorder(c, out);
        }
    }

    /// Every descendant of `id` (excluding it), document order.
    pub fn descendants(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        for c in self.kids(id) {
            self.preorder(c, &mut out);
        }
        out
    }

    pub fn is_in_subtree(&self, root: &str, id: &str) -> bool {
        let mut cur = id;
        loop {
            if cur == root {
                return true;
            }
            if cur == ROOT {
                return false;
            }
            cur = &self.nodes[cur].parent;
        }
    }

    pub fn append(&mut self, node: MNode) {
        self.children
            .entry(node.parent.clone())
            .or_default()
            .push(node.id.clone());
        self.nodes.insert(node.id.clone(), node);
    }

    fn unlink(&mut self, id: &str) {
        let parent = self.nodes[id].parent.clone();
        if let Some(list) = self.children.get_mut(&parent) {
            list.retain(|c| c != id);
            // No empty lists: two trees with the same shape compare equal.
            if list.is_empty() {
                self.children.remove(&parent);
            }
        }
    }

    /// Remove `id` and its whole subtree; returns the removed ids.
    pub fn remove_subtree(&mut self, id: &str) -> Vec<String> {
        let gone = self.subtree(id);
        self.unlink(id);
        for g in &gone {
            self.nodes.remove(g);
            self.children.remove(g);
        }
        gone
    }

    /// Re-home `id` as the LAST child of `new_parent`.
    pub fn move_to(&mut self, id: &str, new_parent: &str) {
        self.unlink(id);
        self.nodes.get_mut(id).unwrap().parent = new_parent.to_string();
        self.children
            .entry(new_parent.to_string())
            .or_default()
            .push(id.to_string());
    }

    /// Place `id` immediately before (or after) its sibling `anchor`.
    pub fn reorder(&mut self, id: &str, anchor: &str, before: bool) {
        let parent = self.nodes[id].parent.clone();
        let list = self.children.get_mut(&parent).unwrap();
        list.retain(|c| c != id);
        let at = list.iter().position(|c| c == anchor).unwrap();
        list.insert(if before { at } else { at + 1 }, id.to_string());
    }
}

/// The model as of one recorded revision of one branch.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub branch: String,
    pub head: HLC,
    /// Index of the op after which this was recorded.
    pub op: usize,
    pub tree: Tree,
    /// Volatile nodes rewritten in place AFTER this snapshot was taken: their
    /// content at `head` is no longer what it was, by design (no history).
    pub tainted: HashSet<String>,
    /// A `main` revision taken between a fork and its merge. Merge replays
    /// the source's entries at their ORIGINAL revisions, so after the merge
    /// these revisions show the fork's changes retroactively; their
    /// mismatches are reported under `checks::name::MERGE_RETRO`.
    pub retro: bool,
    /// Nodes a merge resolution KEPT over the other side's delete, with no
    /// translation written since: merge apply writes no translation overlay
    /// (Phase 2.10's known gap, owned by Phase 11), so their locale reads are
    /// reported under `checks::name::TRANSLATION_AFTER_RESOLUTION`.
    pub overlay_gap: HashSet<String>,
    /// Asked of a stage-3 replica (where translation overlays do not
    /// replicate yet: `checks::name::TRANSLATION_REPLICA`).
    pub replica: bool,
}

/// `(op, id)` events: a resolution that kept a node over a delete (`gap`), or
/// a translation written for it (`healed`).
#[derive(Clone, Debug, Default)]
pub struct OverlayEvents {
    pub gap: Vec<(usize, String)>,
    pub healed: Vec<(usize, String)>,
}

/// Mark each snapshot's overlay gaps from the events — on `main`, and on
/// every branch forked from it after the gap (a fork copies the lost overlay's
/// tombstones along).
pub fn apply_overlay_gaps(snaps: &mut [Snapshot], ev: &OverlayEvents) {
    for s in snaps.iter_mut() {
        for (op, id) in &ev.gap {
            let healed = ev
                .healed
                .iter()
                .any(|(h, hid)| hid == id && h > op && *h <= s.op);
            // A fork taken in the gap's own op is the merge's feature side,
            // which never lost the overlay.
            let after = *op < s.op || (*op == s.op && s.branch == super::env::MAIN);
            if after && !healed {
                s.overlay_gap.insert(id.clone());
            }
        }
    }
}

/// A `versionable=false` update, kept so snapshots can be tainted afterwards.
#[derive(Clone, Debug)]
pub struct InPlaceWrite {
    pub op: usize,
    pub branch: String,
    pub id: String,
}

/// Mark, on every snapshot, the volatile nodes an in-place write rewrote
/// after it. Those are excluded from content assertions at that snapshot —
/// explicitly, by name — instead of passing vacuously.
pub fn apply_taint(snaps: &mut [Snapshot], writes: &[InPlaceWrite]) {
    for s in snaps.iter_mut() {
        for w in writes {
            if w.op > s.op && w.branch == s.branch && s.tree.nodes.contains_key(&w.id) {
                s.tainted.insert(w.id.clone());
            }
        }
    }
}

/// The references a property map holds, at any depth: `(target id, path)`.
pub fn refs_in(props: &HashMap<String, PropertyValue>) -> Vec<(String, String)> {
    fn walk(v: &PropertyValue, out: &mut Vec<(String, String)>) {
        match v {
            PropertyValue::Reference(r) => out.push((r.id.clone(), r.path.clone())),
            PropertyValue::Array(items) => items.iter().for_each(|i| walk(i, out)),
            PropertyValue::Object(m) => m.values().for_each(|i| walk(i, out)),
            PropertyValue::Element(e) => e.content.values().for_each(|i| walk(i, out)),
            PropertyValue::Composite(c) => c
                .items
                .iter()
                .for_each(|e| e.content.values().for_each(|i| walk(i, out))),
            _ => {}
        }
    }
    let mut out = Vec::new();
    props.values().for_each(|v| walk(v, &mut out));
    out
}

pub fn title(n: &MNode) -> String {
    match n.props.get("title") {
        Some(PropertyValue::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Group ids by their timestamp op sequence, ordered ascending.
pub fn by_stamp<'a>(nodes: impl Iterator<Item = (&'a str, u64)>) -> Vec<Vec<String>> {
    let mut groups: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    for (id, seq) in nodes {
        groups.entry(seq).or_default().push(id.to_string());
    }
    groups
        .into_values()
        .map(|mut g| {
            g.sort();
            g
        })
        .collect()
}
