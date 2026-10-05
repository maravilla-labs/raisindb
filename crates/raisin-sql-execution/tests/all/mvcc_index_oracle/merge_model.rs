//! The model side of a merge: three-way by node, sibling order per parent.

use super::model::{MNode, Tree, ROOT};
use std::collections::{BTreeSet, HashSet};

/// What one side of a fork did: nodes it changed, parents whose child list
/// it changed (creates and deletes), nodes it created.
#[derive(Default)]
pub struct Side {
    pub changed: HashSet<String>,
    pub structural: HashSet<String>,
    pub created: HashSet<String>,
}

/// The three-way merge of `main` (ours) and `feat` (theirs) over `base`.
pub fn merged(
    base: &Tree,
    main: &Tree,
    feat: &Tree,
    fs: &Side,
    conflicts: &BTreeSet<String>,
    keep_ours: &HashSet<String>,
) -> Tree {
    let mut all: BTreeSet<String> = base.nodes.keys().cloned().collect();
    all.extend(main.nodes.keys().cloned());
    all.extend(feat.nodes.keys().cloned());
    let chosen = |id: &String| -> Option<MNode> {
        let theirs = if conflicts.contains(id) {
            !keep_ours.contains(id)
        } else {
            fs.changed.contains(id)
        };
        if theirs {
            feat.get(id).cloned()
        } else {
            main.get(id).cloned()
        }
    };
    let mut out = Tree::default();
    for id in &all {
        if let Some(n) = chosen(id) {
            out.nodes.insert(id.clone(), n);
        }
    }
    let mut parents: BTreeSet<String> = out.nodes.values().map(|n| n.parent.clone()).collect();
    parents.insert(ROOT.to_string());
    for p in parents {
        let side = if fs.structural.contains(&p) {
            feat
        } else {
            main
        };
        let mut list: Vec<String> = side
            .kids(&p)
            .iter()
            .filter(|c| out.nodes.get(*c).is_some_and(|n| n.parent == p))
            .cloned()
            .collect();
        // Resurrected by a resolution: back at its base position.
        for (i, c) in base.kids(&p).iter().enumerate() {
            if list.contains(c) || !out.nodes.get(c).is_some_and(|n| n.parent == p) {
                continue;
            }
            let at = base.kids(&p)[..i]
                .iter()
                .rev()
                .find_map(|prev| list.iter().position(|x| x == prev))
                .map_or(0, |pos| pos + 1);
            list.insert(at, c.clone());
        }
        if !list.is_empty() {
            out.children.insert(p, list);
        }
    }
    out
}
