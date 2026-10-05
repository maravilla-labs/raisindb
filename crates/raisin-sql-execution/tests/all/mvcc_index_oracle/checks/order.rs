//! Timestamp-shaped templates: compound CHILD_OF + ORDER BY created_at,
//! ORDER BY created_at/updated_at LIMIT k, timestamp ranges, and COUNT(*).
//!
//! The model knows which OP stamped a timestamp, not its micros, so an order
//! is compared as a sequence of tie GROUPS (ids stamped by the same op may come
//! back in any order among themselves).

use super::tree::{path_of, sample_parents};
use super::{name, sorted, Checker};
use crate::mvcc_index_oracle::env::{column, lit, PAGE, WS};
use crate::mvcc_index_oracle::model::{by_stamp, title, Snapshot};
use serde_json::Value;

/// Whether `got` is a valid answer for `groups` (ascending tie groups, in the
/// requested direction already) under `LIMIT limit`.
pub fn grouped_match(got: &[String], groups: &[Vec<String>], limit: usize) -> bool {
    let total: usize = groups.iter().map(Vec::len).sum();
    if got.len() != total.min(limit) {
        return false;
    }
    let mut at = 0;
    for g in groups {
        if at >= got.len() {
            break;
        }
        let take = g.len().min(got.len() - at);
        let chunk = &got[at..at + take];
        if !chunk.iter().all(|i| g.contains(i)) {
            return false;
        }
        let mut uniq = chunk.to_vec();
        uniq.sort();
        uniq.dedup();
        if uniq.len() != chunk.len() {
            return false;
        }
        at += take;
    }
    true
}

fn count_of(rows: &[Value]) -> Option<i64> {
    let row = rows.first()?.as_object()?;
    let v = row.values().next()?.clone();
    match serde_json::from_value::<raisin_models::nodes::properties::PropertyValue>(v.clone()) {
        Ok(raisin_models::nodes::properties::PropertyValue::Integer(n)) => Some(n),
        _ => v.as_i64(),
    }
}

impl Checker<'_> {
    pub async fn order_templates(&mut self, s: &Snapshot) {
        let rev = self.at(s);
        let only_rev = self.where_rev(s);
        let content_stable = s.tainted.is_empty();

        // Compound (__parent_path, __created_at): a typed folder listing. The
        // planner takes the compound index only with a `node_type =` (a bare
        // CHILD_OF plans as a PrefixScan — the known-failing
        // `compound_index_hierarchy` test on main).
        for p in sample_parents(s, 2) {
            let sql = format!(
                "SELECT id FROM '{WS}' WHERE CHILD_OF('{}') AND node_type = '{PAGE}'{rev} ORDER BY created_at DESC LIMIT 3",
                lit(&path_of(s, &p))
            );
            let typed = s
                .tree
                .kids(&p)
                .iter()
                .filter(|c| s.tree.nodes[*c].node_type == PAGE);
            let mut groups = by_stamp(typed.map(|c| (c.as_str(), s.tree.nodes[c].created)));
            groups.reverse();
            // Asserted exactly at every revision (Phase 8: the reader decides
            // each `(tuple, node)` as of the read revision, and no writer
            // overwrites an entry in place any more).
            let template = if self.at_head {
                name::COMPOUND
            } else {
                name::COMPOUND_HISTORICAL
            };
            self.grouped(template, s, &sql, &groups, 3).await;
        }

        // ORDER BY created_at / updated_at, both directions, LIMIT 4.
        for col in ["created_at", "updated_at"] {
            if col == "updated_at" && !content_stable {
                continue;
            }
            let groups = by_stamp(s.tree.nodes.values().map(|n| {
                (
                    n.id.as_str(),
                    if col == "created_at" {
                        n.created
                    } else {
                        n.updated
                    },
                )
            }));
            for dir in ["ASC", "DESC"] {
                let sql = format!("SELECT id FROM '{WS}'{only_rev} ORDER BY {col} {dir} LIMIT 4");
                let mut g = groups.clone();
                if dir == "DESC" {
                    g.reverse();
                }
                self.grouped(name::TS_ORDER, s, &sql, &g, 4).await;
            }
        }

        // Ranges between recorded instants.
        if self.instants.len() >= 2 {
            let n = self.instants.len();
            let (mut a, mut b) = (
                self.instants[(s.op * 7) % n].clone(),
                self.instants[(s.op * 13 + 3) % n].clone(),
            );
            if a.0 > b.0 {
                std::mem::swap(&mut a, &mut b);
            }
            for col in ["created_at", "updated_at"] {
                if (col == "updated_at" && !content_stable) || a.0 == b.0 {
                    continue;
                }
                let sql = format!(
                    "SELECT id FROM '{WS}' WHERE {col} >= '{}'::TIMESTAMPTZ AND {col} < '{}'::TIMESTAMPTZ{rev}",
                    a.1, b.1
                );
                let want: Vec<String> = s
                    .tree
                    .nodes
                    .values()
                    .filter(|n| {
                        let t = if col == "created_at" {
                            n.created
                        } else {
                            n.updated
                        };
                        t >= a.0 && t < b.0
                    })
                    .map(|n| n.id.clone())
                    .collect();
                match self.sql(s, &sql).await {
                    Ok(rows) => self.expect_eq(
                        name::TS_RANGE,
                        s,
                        &sql,
                        sorted(column(&rows, "id")),
                        sorted(want),
                    ),
                    Err(e) => self.report(name::TS_RANGE, s, format!("{sql}: {e}")),
                }
            }
        }

        // COUNT(*): everything, by type, and by a property (content-dependent).
        let mut counts = vec![
            (
                format!("SELECT COUNT(*) FROM '{WS}'{only_rev}"),
                s.tree.nodes.len(),
            ),
            (
                format!("SELECT COUNT(*) FROM '{WS}' WHERE node_type = '{PAGE}'{rev}"),
                s.tree
                    .nodes
                    .values()
                    .filter(|n| n.node_type == PAGE)
                    .count(),
            ),
        ];
        if content_stable {
            counts.push((
                format!(
                    "SELECT COUNT(*) FROM '{WS}' WHERE properties->>'title'::String = 'alpha'{rev}"
                ),
                s.tree
                    .nodes
                    .values()
                    .filter(|n| title(n) == "alpha")
                    .count(),
            ));
        }
        for (sql, want) in counts {
            match self.sql(s, &sql).await {
                Ok(rows) => {
                    self.expect_eq(name::COUNT, s, &sql, count_of(&rows), Some(want as i64))
                }
                Err(e) => self.report(name::COUNT, s, format!("{sql}: {e}")),
            }
        }
    }

    async fn grouped(
        &mut self,
        template: &'static str,
        s: &Snapshot,
        sql: &str,
        groups: &[Vec<String>],
        limit: usize,
    ) {
        match self.sql(s, sql).await {
            Ok(rows) => {
                let got = column(&rows, "id");
                if !grouped_match(&got, groups, limit) {
                    self.report(
                        template,
                        s,
                        format!("{sql}: got {got:?}, model tie-groups {groups:?}"),
                    );
                }
            }
            Err(e) => self.report(template, s, format!("{sql}: {e}")),
        }
    }
}
