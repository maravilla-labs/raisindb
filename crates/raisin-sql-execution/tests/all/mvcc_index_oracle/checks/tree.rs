//! Tree-shaped templates: path lookup, child listing, editorial order, subtree
//! order with keyset pages, and the next append label.

use super::{name, Checker};
use crate::mvcc_index_oracle::env::column;
use crate::mvcc_index_oracle::model::{Snapshot, ROOT};
use raisin_storage::{ListOptions, NodeRepository, Storage};

/// Parents a per-parent SQL template visits: the root plus a bounded,
/// deterministic, rotating handful (cost control; every parent is still
/// visited by the cheap repository templates).
pub fn sample_parents(s: &Snapshot, cap: usize) -> Vec<String> {
    let mut with_kids: Vec<String> = s
        .tree
        .children
        .iter()
        .filter(|(p, k)| !k.is_empty() && p.as_str() != ROOT && !s.tainted.contains(*p))
        .map(|(p, _)| p.clone())
        .collect();
    with_kids.sort();
    let mut out = vec![ROOT.to_string()];
    if !with_kids.is_empty() {
        let start = s.op % with_kids.len();
        out.extend(
            with_kids
                .iter()
                .cycle()
                .skip(start)
                .take(cap.min(with_kids.len()))
                .cloned(),
        );
    }
    out
}

/// `(id, path)` of every row.
pub fn pairs(s: &Snapshot, rows: &[serde_json::Value]) -> Vec<(String, String)> {
    column(rows, "id")
        .into_iter()
        .zip(column(rows, "path"))
        .map(|(id, p)| {
            let p = no_history(s, &id, p);
            (id, p)
        })
        .collect()
}

/// A `versionable=false` node rewritten in place after the snapshot carries
/// its CURRENT path in the rewritten blob, so its path at that revision is
/// gone by design (no history). Blanked on both sides, by name.
pub fn no_history(s: &Snapshot, id: &str, path: String) -> String {
    if s.tainted.contains(id) {
        "<no history>".to_string()
    } else {
        path
    }
}

pub fn path_of(s: &Snapshot, id: &str) -> String {
    if id == ROOT {
        ROOT.to_string()
    } else {
        s.tree.path(id)
    }
}

impl Checker<'_> {
    pub async fn tree_templates(&mut self, s: &Snapshot) {
        let scope = self.env.scope(&s.branch);
        let nodes = self.env.storage.nodes();

        // get_by_path: every live path resolves to its node; every path this
        // branch ever used and no longer holds resolves to nothing.
        for (id, m) in &s.tree.nodes {
            if s.tainted.contains(id) {
                // Rewritten in place after this snapshot: the rewrite lands
                // its CURRENT path in PATH_INDEX at its old revision. No
                // history, by design.
                continue;
            }
            let path = s.tree.path(id);
            let got = nodes.get_by_path(scope, &path, Some(&s.head)).await;
            let got =
                got.map(|o| o.map(|n| (n.id, n.name, no_history(s, id, n.path), n.has_children)));
            let want = Some((
                id.clone(),
                m.name.clone(),
                no_history(s, id, path.clone()),
                Some(!s.tree.kids(id).is_empty()),
            ));
            self.expect_eq(
                name::GET_BY_PATH,
                s,
                &format!("get_by_path({path})"),
                got.ok().flatten(),
                want,
            );
        }
        let live: std::collections::BTreeSet<String> =
            s.tree.nodes.keys().map(|i| s.tree.path(i)).collect();
        let vacated: Vec<String> = self
            .seen_paths
            .get(&s.branch)
            .map(|all| all.difference(&live).cloned().collect())
            .unwrap_or_default();
        for path in vacated {
            let got = nodes
                .get_node_id_by_path(scope, &path, Some(&s.head))
                .await
                .ok()
                .flatten()
                .filter(|id| !s.tainted.contains(id));
            self.expect_eq(name::GET_BY_PATH, s, &format!("vacated {path}"), got, None);
        }

        // list_by_parent + has_children, for the root and every live node.
        let mut parents = vec![ROOT.to_string()];
        parents.extend(s.tree.ids());
        for p in &parents {
            let options = ListOptions {
                compute_has_children: true,
                max_revision: Some(s.head),
                skip_properties: true,
            };
            let got = nodes.list_by_parent(scope, p, options).await;
            // A tainted node's has_children at a past revision goes through its
            // (rewritten) path: no history, so it is not asserted.
            let hc = |id: &str, v: Option<bool>| if s.tainted.contains(id) { None } else { v };
            let mut got: Vec<(String, Option<bool>)> = match got {
                Ok(v) => v
                    .into_iter()
                    .map(|n| {
                        let h = hc(&n.id, n.has_children);
                        (n.id, h)
                    })
                    .collect(),
                Err(e) => {
                    self.report(
                        name::LIST_BY_PARENT,
                        s,
                        format!("list_by_parent({p}) failed: {e}"),
                    );
                    continue;
                }
            };
            got.sort();
            let mut want: Vec<(String, Option<bool>)> = s
                .tree
                .kids(p)
                .iter()
                .map(|c| (c.clone(), hc(c, Some(!s.tree.kids(c).is_empty()))))
                .collect();
            want.sort();
            self.expect_eq(
                name::LIST_BY_PARENT,
                s,
                &format!("list_by_parent({p})"),
                got,
                want,
            );
            // The ordered-children reader itself, both directions.
            if !s.tree.kids(p).is_empty() {
                let mut want: Vec<String> = s.tree.kids(p).to_vec();
                for descending in [false, true] {
                    let got: Vec<String> = nodes
                        .list_ordered_children_page(scope, p, None, None, descending, Some(&s.head))
                        .await
                        .map(|v| v.into_iter().map(|c| c.child_id).collect())
                        .unwrap_or_default();
                    let what = format!("list_ordered_children_page({p}, descending={descending})");
                    self.expect_eq(name::CHILD_OF_ORDER, s, &what, got, want.clone());
                    want.reverse();
                }
            }
            if p != ROOT && !s.tainted.contains(p) {
                let has = nodes.has_children(scope, p, Some(&s.head)).await.ok();
                let want = Some(!s.tree.kids(p).is_empty());
                self.expect_eq(
                    name::LIST_BY_PARENT,
                    s,
                    &format!("has_children({p})"),
                    has,
                    want,
                );
            }
        }

        self.sql_tree_templates(s).await;

        if self.at_head {
            self.next_append_label(s).await;
        }
    }
}
