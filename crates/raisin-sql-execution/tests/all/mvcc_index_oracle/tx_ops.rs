//! `Op::Tx`: several writes in ONE transaction.
//!
//! Every other op commits one write per transaction, which hides everything
//! that only goes wrong when writes SHARE one: two `versionable=false` nodes
//! rewritten in place at two different revisions (one replicated op used to
//! carry both at one revision), or a create and a move appending under the
//! same parent (two minters used to hand out the same label).
//!
//! Each node is touched at most once per transaction. A move reads its
//! subtree from COMMITTED state, ignoring the transaction's own earlier moves
//! (a known gap of the transactional move, not modelled: moving a child out
//! and then its old parent elsewhere in one transaction rewrites the child's
//! path as if it had stayed). So a move never carries a node this transaction
//! created, wrote or moved — in its subtree as it is now or as it was
//! committed — nor a parent this transaction created under; a create never
//! lands under a subtree this transaction moved. After a move, content writes
//! in the same transaction carry no references (`content`).

use super::driver::Run;
use super::env::{MAIN, PAGE, WS};
use super::model::{InPlaceWrite, MNode, ROOT};
use super::ops::{name_of, Props, TxStep};
use std::collections::HashSet;

#[derive(Default)]
struct Touched {
    /// Written, created, or carried by a move.
    nodes: HashSet<String>,
    /// Parents this transaction created under (a move must not carry them).
    create_parents: HashSet<String>,
    /// Whether a move already ran in this transaction.
    moved: bool,
}

/// After a move in the same transaction, content writes carry no references:
/// the write path resolves a reference's path against COMMITTED state, not
/// the transaction's own moves (a known gap, not modelled).
fn content(props: &Props, t: &Touched) -> Props {
    if t.moved {
        Props {
            link: None,
            card: None,
            ..props.clone()
        }
    } else {
        props.clone()
    }
}

impl Run {
    pub async fn apply_tx(&mut self, steps: &[TxStep]) -> Option<String> {
        let pre: HashSet<String> = self.scoped_ids().into_iter().collect();
        let pre_tree = self.tree.clone();
        let mut t = Touched::default();
        let ctx = self.env.tx(MAIN).await;
        let mut descs = Vec::new();
        let mut in_place = true;
        for step in steps {
            // Pre-existing and untouched: the only nodes a step may land on.
            let free: Vec<String> = self
                .scoped_ids()
                .into_iter()
                .filter(|i| pre.contains(i) && !t.nodes.contains(i))
                .collect();
            let res = match step {
                TxStep::Volatile { target, props } | TxStep::Update { target, props } => {
                    let volatile = matches!(step, TxStep::Volatile { .. });
                    let cands: Vec<String> = free
                        .iter()
                        .filter(|i| Self::page_like(&self.tree.nodes[*i]) != volatile)
                        .cloned()
                        .collect();
                    let Some(id) = Self::pick(&cands, *target).cloned() else {
                        continue;
                    };
                    let mut props = Self::build_props(&self.tree, &id, &content(props, &t));
                    Self::refresh_refs(&self.tree, &mut props);
                    let n = self.tree.nodes.get_mut(&id).unwrap();
                    n.props = props.clone();
                    n.updated = self.seq;
                    t.nodes.insert(id.clone());
                    if volatile {
                        self.writes.push(InPlaceWrite {
                            op: self.op_index,
                            branch: MAIN.to_string(),
                            id: id.clone(),
                        });
                    } else {
                        in_place = false;
                    }
                    descs.push(format!(
                        "{} {id}",
                        if volatile { "volatile" } else { "update" }
                    ));
                    match ctx.get_node(WS, &id).await {
                        Ok(Some(mut current)) => {
                            current.properties = props;
                            ctx.put_node(WS, &current).await
                        }
                        other => Err(raisin_error::Error::Backend(format!("tx read: {other:?}"))),
                    }
                }
                TxStep::Create {
                    parent,
                    name,
                    props,
                } => {
                    let parent = self.tx_parent(&free, *parent, &t);
                    let name = name_of(*name);
                    if self.tree.name_taken(&parent, &name) {
                        continue;
                    }
                    let id = self.fresh_id();
                    let props = Self::build_props(&self.tree, &id, &content(props, &t));
                    self.tree.append(MNode {
                        id: id.clone(),
                        name,
                        parent: parent.clone(),
                        node_type: PAGE.to_string(),
                        props,
                        created: self.seq,
                        updated: self.seq,
                        overlay: None,
                    });
                    t.nodes.insert(id.clone());
                    t.create_parents.insert(parent);
                    in_place = false;
                    let node = Self::wire_node(&self.tree, &id);
                    descs.push(format!("create {id} at {}", node.path));
                    ctx.add_node(WS, &node).await
                }
                TxStep::MoveInto { target, parent } => {
                    // Neither the subtree as it stands now nor as it was
                    // committed may hold anything this transaction touched:
                    // the move rewrites the COMMITTED subtree's paths (see
                    // the module docs).
                    let clean = |tree: &super::model::Tree, i: &String| {
                        tree.subtree(i)
                            .iter()
                            .all(|d| !t.nodes.contains(d) && !t.create_parents.contains(d))
                    };
                    let cands: Vec<String> = free
                        .iter()
                        .filter(|i| clean(&self.tree, i) && clean(&pre_tree, i))
                        .cloned()
                        .collect();
                    let Some(id) = Self::pick(&cands, *target).cloned() else {
                        continue;
                    };
                    let to = self.tx_parent(&free, *parent, &t);
                    let n = &self.tree.nodes[&id];
                    if to == n.parent
                        || (to != ROOT && self.tree.is_in_subtree(&id, &to))
                        || self.tree.name_taken(&to, &n.name)
                    {
                        continue;
                    }
                    let from = self.tree.path(&id);
                    self.tree.move_to(&id, &to);
                    self.tree.nodes.get_mut(&id).unwrap().updated = self.seq;
                    t.nodes.extend(self.tree.subtree(&id));
                    t.moved = true;
                    in_place = false;
                    let new_path = self.tree.path(&id);
                    descs.push(format!("move {id} {from} -> {new_path}"));
                    ctx.move_node_tree(WS, &id, &new_path).await
                }
            };
            if let Err(e) = res {
                let desc = format!("tx [{}]", descs.join("; "));
                return self.fail(&desc, e);
            }
        }
        if descs.is_empty() {
            return None;
        }
        self.last_in_place = in_place;
        let desc = format!("tx [{}]", descs.join("; "));
        match ctx.commit().await {
            Ok(()) => Some(desc),
            Err(e) => self.fail(&desc, e),
        }
    }

    /// A parent for a create or a move destination: a pre-existing node no
    /// move in this transaction carried (its path changed mid-transaction), or
    /// the scope's top.
    fn tx_parent(&self, free: &[String], sel: Option<u16>, t: &Touched) -> String {
        let top = self.scope.clone().unwrap_or_else(|| ROOT.to_string());
        let cands: Vec<String> = free
            .iter()
            .filter(|i| !t.nodes.contains(*i))
            .cloned()
            .collect();
        match sel {
            None => top,
            Some(s) => Self::pick(&cands, s).cloned().unwrap_or(top),
        }
    }
}
