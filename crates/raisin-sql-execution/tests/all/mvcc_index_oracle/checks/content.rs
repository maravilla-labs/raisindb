//! Content templates: property equality, `node_type =`, REFERENCES,
//! RESOLVE(properties, 2) and a locale read.
//!
//! Volatile nodes rewritten in place after the snapshot (`Snapshot.tainted`)
//! are dropped from both sides of every comparison here, by name.

use super::{name, sorted, untainted, Checker};
use crate::mvcc_index_oracle::env::{column, lit, DOC, LOCALE, PAGE, VOLATILE, WS};
use crate::mvcc_index_oracle::model::{refs_in, title, Overlay, Snapshot};
use crate::mvcc_index_oracle::ops::TITLES;
use raisin_models::nodes::properties::PropertyValue;
use serde_json::Value;

/// What a RESOLVE'd reference looks like once projected: inlined (with the
/// target's id, title and its own projected `link`) or left bare.
#[derive(Debug, PartialEq)]
pub enum Proj {
    Bare(String),
    Inlined {
        id: String,
        title: String,
        link: Option<Box<Proj>>,
    },
}

fn project_json(v: &Value) -> Option<Proj> {
    let o = v.as_object()?;
    if let Some(r) = o.get("raisin:ref") {
        return Some(Proj::Bare(r.as_str().unwrap_or_default().to_string()));
    }
    Some(Proj::Inlined {
        id: o.get("id")?.as_str()?.to_string(),
        title: o
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        link: o.get("link").and_then(project_json).map(Box::new),
    })
}

fn project_model(s: &Snapshot, target: &str, depth: u32) -> Proj {
    match s.tree.get(target) {
        Some(n) if depth > 0 => Proj::Inlined {
            id: n.id.clone(),
            title: title(n),
            link: match n.props.get("link") {
                Some(PropertyValue::Reference(r)) => {
                    Some(Box::new(project_model(s, &r.id, depth - 1)))
                }
                _ => None,
            },
        },
        _ => Proj::Bare(target.to_string()),
    }
}

impl Checker<'_> {
    pub async fn content_templates(&mut self, s: &Snapshot) {
        let rev = self.at(s);
        let ids_where =
            |f: &dyn Fn(&crate::mvcc_index_oracle::model::MNode) -> bool| -> Vec<String> {
                sorted(untainted(
                    s,
                    s.tree
                        .nodes
                        .values()
                        .filter(|n| f(n))
                        .map(|n| n.id.clone())
                        .collect(),
                ))
            };

        let mut cases: Vec<(&'static str, String, Vec<String>)> = Vec::new();
        for t in TITLES {
            cases.push((
                name::PROPERTY_EQ,
                format!("SELECT id FROM '{WS}' WHERE properties->>'title'::String = '{t}'{rev}"),
                ids_where(&|n| title(n) == t),
            ));
        }
        for r in [-1i64, 2] {
            cases.push((
                name::PROPERTY_EQ,
                format!("SELECT id FROM '{WS}' WHERE properties->>'rank'::String = '{r}'{rev}"),
                ids_where(&|n| n.props.get("rank") == Some(&PropertyValue::Integer(r))),
            ));
        }
        for ty in [PAGE, DOC, VOLATILE] {
            cases.push((
                name::NODE_TYPE_EQ,
                format!("SELECT id FROM '{WS}' WHERE node_type = '{ty}'{rev}"),
                ids_where(&|n| n.node_type == ty),
            ));
        }
        // REFERENCES('ws:/p') names the node that lives at /p at the query's
        // revision; the reverse index is keyed by that node's ID, so a
        // referrer matches by id whatever path its reference recorded.
        let mut targets: Vec<String> = s
            .tree
            .nodes
            .values()
            .filter(|n| !s.tainted.contains(&n.id))
            .flat_map(|n| refs_in(&n.props).into_iter().map(|(id, _)| id))
            .filter(|id| s.tree.nodes.contains_key(id))
            .collect();
        targets.sort();
        targets.dedup();
        let skip = s.op % targets.len().max(1);
        let mut probe: Vec<String> = targets
            .iter()
            .cycle()
            .skip(skip)
            .take(targets.len().min(3))
            .cloned()
            .collect();
        // Plus one node the model says NOTHING references: a stale referrer
        // (a removed reference still live in the index) only shows there.
        let all = s.tree.ids();
        if !all.is_empty() {
            probe.push(all[(s.op * 7) % all.len()].clone());
        }
        probe.dedup();
        for target in &probe {
            let path = s.tree.path(target);
            cases.push((
                name::REFERENCES,
                format!(
                    "SELECT id FROM '{WS}' WHERE REFERENCES('{WS}:{}'){rev}",
                    lit(&path)
                ),
                ids_where(&|n| refs_in(&n.props).iter().any(|(id, _)| id == target)),
            ));
        }
        for (template, sql, want) in cases {
            match self.sql(s, &sql).await {
                Ok(rows) => {
                    let got = sorted(untainted(s, column(&rows, "id")));
                    self.expect_eq(template, s, &sql, got, want);
                }
                Err(e) => self.report(template, s, format!("{sql}: {e}")),
            }
        }

        self.resolve(s, &rev).await;
        self.locale_read(s, &rev).await;
    }

    async fn resolve(&mut self, s: &Snapshot, rev: &str) {
        let linked: Vec<(String, String)> = s
            .tree
            .nodes
            .values()
            .filter_map(|n| match n.props.get("link") {
                Some(PropertyValue::Reference(r)) => Some((n.id.clone(), r.id.clone())),
                _ => None,
            })
            .filter(|(src, dst)| {
                // Any tainted node on the chain makes inlined content unknowable.
                let mut chain = vec![src.clone(), dst.clone()];
                if let Some(PropertyValue::Reference(r)) =
                    s.tree.get(dst).and_then(|d| d.props.get("link"))
                {
                    chain.push(r.id.clone());
                }
                !chain.iter().any(|c| s.tainted.contains(c))
            })
            .collect();
        let skip = s.op % linked.len().max(1);
        for (src, dst) in linked.iter().cycle().skip(skip).take(linked.len().min(2)) {
            let sql =
                format!("SELECT RESOLVE(properties, 2) AS r FROM '{WS}' WHERE id = '{src}'{rev}");
            match self.sql(s, &sql).await {
                Ok(rows) => {
                    let got = rows
                        .first()
                        .and_then(|r| r.get("r"))
                        .and_then(|r| r.get("link"))
                        .and_then(project_json);
                    let want = Some(project_model(s, dst, 2));
                    self.expect_eq(name::RESOLVE, s, &sql, got, want);
                }
                Err(e) => self.report(name::RESOLVE, s, format!("{sql}: {e}")),
            }
        }
    }

    async fn locale_read(&mut self, s: &Snapshot, rev: &str) {
        let sql = format!(
            "SELECT id, properties->>'title'::String AS t FROM '{WS}' WHERE locale = '{LOCALE}'{rev}"
        );
        let mut want: Vec<(String, String)> = s
            .tree
            .nodes
            .values()
            .filter(|n| !s.tainted.contains(&n.id))
            .filter_map(|n| match &n.overlay {
                Some(Overlay::Hidden) => None,
                Some(Overlay::Title(t)) => Some((n.id.clone(), t.clone())),
                None => Some((n.id.clone(), title(n))),
            })
            .collect();
        want.sort();
        match self.sql(s, &sql).await {
            Ok(rows) => {
                let mut got: Vec<(String, String)> = column(&rows, "id")
                    .into_iter()
                    .zip(column(&rows, "t"))
                    .filter(|(id, _)| !s.tainted.contains(id))
                    .collect();
                got.sort();
                let template = if s.replica {
                    name::TRANSLATION_REPLICA
                } else if self.at_head {
                    name::TRANSLATION
                } else {
                    name::TRANSLATION_HISTORICAL
                };
                // Split per node so a known gap names only its own nodes.
                let gap = |id: &String| self.at_head && s.overlay_gap.contains(id);
                let (gap_got, got): (Vec<_>, Vec<_>) = got.into_iter().partition(|(id, _)| gap(id));
                let (gap_want, want): (Vec<_>, Vec<_>) =
                    want.into_iter().partition(|(id, _)| gap(id));
                self.expect_eq(template, s, &sql, got, want);
                if self.at_head {
                    self.expect_eq(
                        name::TRANSLATION_AFTER_RESOLUTION,
                        s,
                        &format!("{sql} (nodes kept over a delete)"),
                        gap_got,
                        gap_want,
                    );
                }
            }
            Err(e) => self.report(name::TRANSLATION, s, format!("{sql}: {e}")),
        }
    }
}
