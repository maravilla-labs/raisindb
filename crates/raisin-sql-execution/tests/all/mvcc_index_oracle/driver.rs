//! Drives one generated history through the real write funnels while
//! advancing the reference model by the same op, and records a snapshot of
//! the model at every revision the branch HEAD reaches.

use super::env::{Env, MAIN, WS};
use super::model::{InPlaceWrite, MNode, Snapshot, Tree, ROOT};
use super::ops::{Op, Props, TITLES};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::value::{Composite, Element};
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_models::nodes::Node;
use std::collections::HashMap;

pub struct Run {
    pub env: Env,
    /// The model of `main`.
    pub tree: Tree,
    /// Op sequence: every op that stamps a timestamp stamps THIS value.
    pub seq: u64,
    pub snaps: Vec<Snapshot>,
    pub writes: Vec<InPlaceWrite>,
    /// `(seq, instant)`: a wall-clock instant strictly between op `seq - 1`
    /// and op `seq`, RFC 3339 with nanoseconds.
    pub instants: Vec<(u64, String)>,
    pub log: Vec<String>,
    /// Problems found while DRIVING (a write that minted no revision, a merge
    /// whose conflict set differs from the model's). Reported with mismatches.
    pub anomalies: Vec<String>,
    pub next_id: usize,
    pub forks: usize,
    pub op_index: usize,
    pub overlay_events: super::model::OverlayEvents,
    /// Stage 3: this origin's own folder; ops stay beneath it.
    pub scope: Option<String>,
    /// Prefix of minted ids (`o` by default; one per stage-3 origin).
    pub id_prefix: &'static str,
    /// Whether the last `Op::Tx` / `Op::ForkMerge` changed the model only by
    /// in-place (`versionable=false`) writes, so HEAD is expected not to move.
    pub last_in_place: bool,
    /// Mismatches found by checking a fork window's `main` snapshots just
    /// before its merge (when they are still the truth). Strict: unlike the
    /// same snapshots after the merge, these are never `retro`.
    pub pre_merge: Vec<super::checks::Mismatch>,
}

impl Run {
    pub async fn new(env: Env) -> Self {
        let head = env.head(MAIN).await;
        let snaps = vec![Snapshot {
            branch: MAIN.to_string(),
            head,
            op: 0,
            tree: Tree::default(),
            tainted: Default::default(),
            retro: false,
            overlay_gap: Default::default(),
            replica: false,
        }];
        Self {
            env,
            tree: Tree::default(),
            seq: 0,
            snaps,
            writes: Vec::new(),
            instants: Vec::new(),
            log: Vec::new(),
            anomalies: Vec::new(),
            next_id: 0,
            forks: 0,
            op_index: 0,
            overlay_events: Default::default(),
            scope: None,
            id_prefix: "o",
            last_in_place: false,
            pre_merge: Vec::new(),
        }
    }

    pub fn fresh_id(&mut self) -> String {
        self.next_id += 1;
        format!("{}{:03}", self.id_prefix, self.next_id)
    }

    /// The ids ops may land on: everything, or — for a stage-3 origin — only
    /// the subtree below its own folder (the folder itself excluded).
    pub fn scoped_ids(&self) -> Vec<String> {
        match &self.scope {
            None => self.tree.ids(),
            Some(root) => self
                .tree
                .ids()
                .into_iter()
                .filter(|i| i != root && self.tree.is_in_subtree(root, i))
                .collect(),
        }
    }

    /// Resolve a selector against a candidate list.
    pub fn pick<'a>(cands: &'a [String], sel: u16) -> Option<&'a String> {
        if cands.is_empty() {
            None
        } else {
            Some(&cands[sel as usize % cands.len()])
        }
    }

    /// The property map a write sets, built from the op and the model.
    pub fn build_props(tree: &Tree, id: &str, p: &Props) -> HashMap<String, PropertyValue> {
        let ids = tree.ids();
        let reference = |sel: u16| {
            Self::pick(&ids, sel).map(|t| {
                PropertyValue::Reference(RaisinReference {
                    id: t.clone(),
                    workspace: super::env::WS.to_string(),
                    path: tree.path(t),
                })
            })
        };
        let mut out = HashMap::new();
        out.insert(
            "title".to_string(),
            PropertyValue::String(TITLES[p.title as usize % TITLES.len()].to_string()),
        );
        if let Some(rank) = p.rank {
            out.insert("rank".to_string(), PropertyValue::Integer(rank as i64));
        }
        if let Some(r) = p.link.and_then(reference) {
            out.insert("link".to_string(), r);
        }
        if let Some((composite, sel)) = p.card {
            if let Some(r) = reference(sel) {
                let element = Element {
                    uuid: format!("card-{id}"),
                    element_type: "oracle:Card".to_string(),
                    content: HashMap::from([
                        ("label".to_string(), PropertyValue::String("card".into())),
                        ("target".to_string(), r),
                    ]),
                };
                let value = if composite {
                    PropertyValue::Composite(Composite {
                        uuid: format!("blocks-{id}"),
                        items: vec![element],
                    })
                } else {
                    PropertyValue::Element(element)
                };
                out.insert("card".to_string(), value);
            }
        }
        out
    }

    /// The write path re-resolves every reference's `raisin:path` to its
    /// target's CURRENT path (the denormalized path is refreshed on write), so
    /// content the model writes carries the targets' paths as of the write.
    pub fn refresh_refs(tree: &Tree, props: &mut HashMap<String, PropertyValue>) {
        fn walk(tree: &Tree, v: &mut PropertyValue) {
            match v {
                PropertyValue::Reference(r) => {
                    if tree.nodes.contains_key(&r.id) {
                        r.path = tree.path(&r.id);
                    }
                }
                PropertyValue::Array(items) => items.iter_mut().for_each(|i| walk(tree, i)),
                PropertyValue::Object(m) => m.values_mut().for_each(|i| walk(tree, i)),
                PropertyValue::Element(e) => e.content.values_mut().for_each(|i| walk(tree, i)),
                PropertyValue::Composite(c) => c
                    .items
                    .iter_mut()
                    .for_each(|e| e.content.values_mut().for_each(|i| walk(tree, i))),
                _ => {}
            }
        }
        props.values_mut().for_each(|v| walk(tree, v));
    }

    /// Record a wall-clock instant between the previous op and the next.
    async fn mark_instant(&mut self) {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
        self.instants.push((self.seq, now));
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }

    /// Apply one op to the system and the model, then record a snapshot.
    pub async fn apply(&mut self, op: &Op) {
        self.op_index += 1;
        self.seq += 1;
        self.mark_instant().await;
        let before = self.tree.clone();
        let anomalies = self.anomalies.len();
        let applied = match op {
            // A stage-3 origin replicates one branch; forks are stage 1's.
            Op::ForkMerge(_) if self.scope.is_some() => None,
            Op::ForkMerge(spec) => self.fork_merge(spec).await,
            other => self.apply_simple(other).await,
        };
        let Some(desc) = applied else {
            self.log.push(format!("#{} skipped {op:?}", self.op_index));
            return;
        };
        self.log.push(format!("#{} {desc}", self.op_index));
        if self.anomalies.len() > anomalies && !matches!(op, Op::ForkMerge(_)) {
            // The system refused an op the model accepted. That is already
            // reported; un-applying it in the model keeps one finding from
            // cascading into every later comparison.
            self.tree = before;
            return;
        }
        let in_place = match op {
            Op::Volatile { .. } => true,
            Op::Tx(_) | Op::ForkMerge(_) => self.last_in_place,
            _ => false,
        };
        self.record(&before, in_place).await;
    }

    /// Snapshot `main` if its HEAD moved. An in-place (`versionable=false`)
    /// write mints nothing, so the newest snapshot is brought up to date
    /// instead: reading at that revision now returns the rewritten content.
    async fn record(&mut self, before: &Tree, in_place: bool) {
        let head = self.env.head(MAIN).await;
        let last = self
            .snaps
            .iter()
            .rposition(|s| s.branch == MAIN)
            .expect("initial snapshot");
        if head != self.snaps[last].head {
            self.snaps.push(Snapshot {
                branch: MAIN.to_string(),
                head,
                op: self.op_index,
                tree: self.tree.clone(),
                tainted: Default::default(),
                retro: false,
                overlay_gap: Default::default(),
                replica: false,
            });
        } else if self.tree != *before {
            if !in_place {
                self.anomalies.push(format!(
                    "op #{} changed the model but HEAD did not move ({head})",
                    self.op_index
                ));
            }
            self.snaps[last].tree = self.tree.clone();
            self.snaps[last].op = self.op_index;
        }
    }

    pub fn page_like(n: &MNode) -> bool {
        n.node_type != super::env::VOLATILE
    }

    pub fn parent_sel(&self, sel: Option<u16>) -> String {
        let top = self.scope.clone().unwrap_or_else(|| ROOT.to_string());
        match sel {
            None => top,
            Some(s) => Self::pick(&self.scoped_ids(), s).cloned().unwrap_or(top),
        }
    }

    /// `rev` spelled the way RESTORE's revision grammar wants it.
    pub fn restore_rev(rev: &HLC) -> String {
        format!("{}_{}", rev.timestamp_ms, rev.counter)
    }

    /// The `Node` a client sends for model node `id` on `branch`'s tree.
    pub fn wire_node(tree: &super::model::Tree, id: &str) -> Node {
        let n = &tree.nodes[id];
        let parent_name = if n.parent == ROOT {
            ROOT.to_string()
        } else {
            tree.nodes[&n.parent].name.clone()
        };
        Node {
            id: n.id.clone(),
            name: n.name.clone(),
            path: tree.path(id),
            node_type: n.node_type.clone(),
            parent: Some(parent_name),
            properties: n.props.clone(),
            workspace: Some(WS.to_string()),
            ..Default::default()
        }
    }

    pub fn fail(&mut self, what: &str, e: impl std::fmt::Display) -> Option<String> {
        self.anomalies
            .push(format!("op #{} {what} failed: {e}", self.op_index));
        Some(format!("{what} (FAILED)"))
    }
}
