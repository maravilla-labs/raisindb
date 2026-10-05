//! Localized URL lookup (plan Phase 12): `RESOLVE_PATH(ws, LOCALE, …)` at the
//! snapshot's revision against the model.
//!
//! The op log's French overlay sets `/__node_name` to its title (`write_ops.rs`),
//! so a node's French segment is that title when it has an overlay, its name
//! otherwise, and a node hidden in French — or under a hidden ancestor — has
//! no French path. A segment two visible siblings could both answer (equal
//! translated names; a translated name equal to an untranslated sibling's
//! name) is a collision the
//! model does not arbitrate, and the template skips it. `main` is built at
//! setup, so the index answers there (its inline writers are what is under
//! test); forks and replicas answer through the row-level fallback.

use super::{name, Checker};
use crate::mvcc_index_oracle::env::{column, LOCALE, WS};
use crate::mvcc_index_oracle::model::{Overlay, Snapshot, Tree, ROOT};

/// A node's French segment and whether it is its own translated name; `None`
/// if hidden.
fn segment(tree: &Tree, id: &str) -> Option<(String, bool)> {
    let node = tree.get(id)?;
    match &node.overlay {
        Some(Overlay::Hidden) => None,
        Some(Overlay::Title(t)) => Some((t.clone(), true)),
        None => Some((node.name.clone(), false)),
    }
}

/// Whether any node on `id`'s chain was rewritten in place after the
/// snapshot (`Snapshot.tainted`): a `versionable=false` node's history —
/// its path included — is rewritten by design, so its past is unknowable.
fn chain_tainted(s: &Snapshot, id: &str) -> bool {
    let mut cur = id.to_string();
    while cur != ROOT {
        if s.tainted.contains(&cur) {
            return true;
        }
        let Some(node) = s.tree.get(&cur) else {
            return true;
        };
        cur = node.parent.clone();
    }
    false
}

/// The model's French path of `id`, unless hidden or ambiguous on its chain.
fn localized_path(tree: &Tree, id: &str) -> Option<String> {
    let mut segs = Vec::new();
    let mut cur = id.to_string();
    while cur != ROOT {
        let (seg, own) = segment(tree, &cur)?;
        let parent = tree.get(&cur)?.parent.clone();
        for sibling in tree.kids(&parent).iter().filter(|s| **s != cur) {
            if let Some((other, other_own)) = segment(tree, sibling) {
                // An equal translated name, or one shadowing this node's plain name.
                if other_own && other == seg {
                    return None;
                }
                if !own && other_own && other == tree.get(&cur)?.name {
                    return None;
                }
            }
        }
        segs.push(seg);
        cur = parent;
    }
    segs.reverse();
    Some(format!("/{}", segs.join("/")))
}

impl Checker<'_> {
    pub(super) async fn localized_templates(&mut self, s: &Snapshot) {
        let Some(anchor) = s.tree.nodes.keys().next().map(|id| s.tree.path(id)) else {
            return;
        };
        let rev = self.at(s);
        let ids: Vec<String> = s
            .tree
            .nodes
            .keys()
            .filter(|id| !chain_tainted(s, id))
            .cloned()
            .collect();
        let skip = s.op % ids.len().max(1);
        for id in ids.iter().cycle().skip(skip).take(ids.len().min(3)) {
            let (path, want) = match localized_path(&s.tree, id) {
                Some(p) => (p, id.clone()),
                // Hidden on its chain: its canonical path resolves to nothing.
                None if segment(&s.tree, id).is_none() => (s.tree.path(id), String::new()),
                None => continue,
            };
            let sql = format!(
                "SELECT RESOLVE_PATH('{WS}', '{LOCALE}', '{path}') AS r FROM '{WS}' \
                 WHERE path = '{anchor}'{rev}"
            );
            match self.sql(s, &sql).await {
                Ok(rows) => {
                    let got = column(&rows, "r").into_iter().next().unwrap_or_default();
                    self.expect_eq(name::LOCALIZED_PATH, s, &sql, got, want);
                }
                Err(e) => self.report(name::LOCALIZED_PATH, s, format!("{sql}: {e}")),
            }
        }
    }
}
