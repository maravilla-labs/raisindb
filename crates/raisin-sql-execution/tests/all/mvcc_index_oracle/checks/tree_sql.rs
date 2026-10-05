//! The SQL tree templates — CHILD_OF by `__order`, DESCENDANT_OF by
//! `__tree_order` in keyset pages — and the next-append-label check.

use super::tree::{no_history, pairs, path_of, sample_parents};
use super::{name, Checker};
use crate::mvcc_index_oracle::env::{column, lit, REPO, TENANT, WS};
use crate::mvcc_index_oracle::model::{Snapshot, ROOT};
use raisin_rocksdb::fractional_index;
use raisin_storage::{NodeRepository, Storage};

impl Checker<'_> {
    /// The SQL tree templates (CHILD_OF by `__order`, DESCENDANT_OF pages).
    pub async fn sql_tree_templates(&mut self, s: &Snapshot) {
        // CHILD_OF ... ORDER BY __order, both directions for the root.
        for p in sample_parents(s, 3) {
            let path = path_of(s, &p);
            let base = format!(
                "SELECT id, path FROM '{WS}' WHERE CHILD_OF('{}'){} ORDER BY __order",
                lit(&path),
                self.at(s)
            );
            let want: Vec<(String, String)> = s
                .tree
                .kids(&p)
                .iter()
                .map(|c| (c.clone(), no_history(s, c, s.tree.path(c))))
                .collect();
            match self.sql(s, &base).await {
                Ok(rows) => self.expect_eq(
                    name::CHILD_OF_ORDER,
                    s,
                    &base,
                    pairs(s, &rows),
                    want.clone(),
                ),
                Err(e) => self.report(name::CHILD_OF_ORDER, s, format!("{base}: {e}")),
            }
            if p == ROOT {
                let desc = format!("{base} DESC");
                let mut rev = want;
                rev.reverse();
                match self.sql(s, &desc).await {
                    Ok(rows) => {
                        self.expect_eq(name::CHILD_OF_ORDER, s, &desc, pairs(s, &rows), rev)
                    }
                    Err(e) => self.report(name::CHILD_OF_ORDER, s, format!("{desc}: {e}")),
                }
            }
        }

        // DESCENDANT_OF ... ORDER BY __tree_order, walked in keyset pages of 2.
        let mut tops: Vec<String> = s
            .tree
            .kids(ROOT)
            .iter()
            .filter(|t| !s.tree.kids(t).is_empty() && !s.tainted.contains(*t))
            .cloned()
            .collect();
        tops.truncate(2);
        for top in tops {
            let path = s.tree.path(&top);
            let mut got = Vec::new();
            let mut cursor: Option<String> = None;
            for _ in 0..60 {
                let after = cursor
                    .as_ref()
                    .map(|c| format!(" AND __tree_order > '{}'", lit(c)))
                    .unwrap_or_default();
                let sql = format!(
                    "SELECT id, path, __tree_order FROM '{WS}' WHERE DESCENDANT_OF('{}'){}{after} ORDER BY __tree_order LIMIT 2",
                    lit(&path),
                    self.at(s)
                );
                let rows = match self.sql(s, &sql).await {
                    Ok(r) => r,
                    Err(e) => {
                        self.report(name::DESCENDANT_PAGES, s, format!("{sql}: {e}"));
                        break;
                    }
                };
                got.extend(pairs(s, &rows));
                cursor = column(&rows, "__tree_order").last().cloned();
                if rows.len() < 2 {
                    break;
                }
            }
            let want: Vec<(String, String)> = s
                .tree
                .descendants(&top)
                .into_iter()
                .map(|d| {
                    let p = no_history(s, &d, s.tree.path(&d));
                    (d, p)
                })
                .collect();
            self.expect_eq(
                name::DESCENDANT_PAGES,
                s,
                &format!("DESCENDANT_OF({path}) pages"),
                got,
                want,
            );
        }
    }

    /// At HEAD: an append under any parent must land after every live child.
    pub async fn next_append_label(&mut self, s: &Snapshot) {
        let nodes = self.env.storage.nodes();
        let mut parents: Vec<String> = s.tree.children.keys().cloned().collect();
        parents.sort();
        for p in parents {
            if s.tree.kids(&p).is_empty() {
                continue;
            }
            let last = self
                .env
                .storage
                .nodes_impl()
                .last_order_label_for_append(TENANT, REPO, &s.branch, WS, &p);
            let next = match last {
                Ok(Some(l)) => fractional_index::inc(fractional_index::extract_fractional(&l)).ok(),
                _ => None,
            };
            let Some(next) = next else {
                self.report(
                    name::LAST_LABEL,
                    s,
                    format!("parent {p}: no last label to append after"),
                );
                continue;
            };
            let page = nodes
                .list_ordered_children_page(
                    self.env.scope(&s.branch),
                    &p,
                    None,
                    None,
                    false,
                    Some(&s.head),
                )
                .await
                .unwrap_or_default();
            for c in page {
                let frac = fractional_index::extract_fractional(&c.order_label);
                if frac >= next.as_str() && s.tree.nodes.contains_key(&c.child_id) {
                    self.report(
                        name::LAST_LABEL,
                        s,
                        format!("parent {p}: next append label {next} does not sort after live child {} ({frac})", c.child_id),
                    );
                }
            }
        }
    }
}
