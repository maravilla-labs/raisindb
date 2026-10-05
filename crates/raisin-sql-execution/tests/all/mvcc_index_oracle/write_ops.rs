//! The non-branching ops: each one checks its precondition against the model,
//! performs the write through a real funnel, and advances the model.
//!
//! A system call that FAILS is not a skipped op: it is recorded as an
//! anomaly, because the model already decided the op was valid.

use super::driver::Run;
use super::env::{DOC, LOCALE, MAIN, PAGE, VOLATILE, WS};
use super::model::{MNode, Overlay, ROOT};
use super::ops::{name_of, Funnel, Op, TITLES};
use raisin_models::translations::{JsonPointer, LocaleOverlay};
use raisin_storage::{DeleteNodeOptions, NodeRepository, Storage};
use std::collections::HashMap;

impl Run {
    pub async fn apply_simple(&mut self, op: &Op) -> Option<String> {
        let ids = self.scoped_ids();
        let seq = self.seq;
        match op {
            Op::Create {
                parent,
                name,
                ty,
                props,
            } => {
                let parent = self.parent_sel(*parent);
                let name = name_of(*name);
                if self.tree.name_taken(&parent, &name) {
                    return None;
                }
                let id = self.fresh_id();
                let node_type = match ty % 6 {
                    0..=2 => PAGE,
                    3 | 4 => DOC,
                    _ => VOLATILE,
                };
                let props = Self::build_props(&self.tree, &id, props);
                self.tree.append(MNode {
                    id: id.clone(),
                    name,
                    parent,
                    node_type: node_type.to_string(),
                    props,
                    created: seq,
                    updated: seq,
                    overlay: None,
                });
                let node = Self::wire_node(&self.tree, &id);
                let tx = self.env.tx(MAIN).await;
                let res = match tx.add_node(WS, &node).await {
                    Ok(()) => tx.commit().await,
                    Err(e) => Err(e),
                };
                let desc = format!("create {id} at {} ({node_type})", node.path);
                match res {
                    Ok(()) => Some(desc),
                    Err(e) => self.fail(&desc, e),
                }
            }
            Op::Update {
                target,
                props,
                funnel,
            } => {
                let cands: Vec<String> = ids
                    .iter()
                    .filter(|i| Self::page_like(&self.tree.nodes[*i]))
                    .cloned()
                    .collect();
                let id = Self::pick(&cands, *target)?.clone();
                let props = Self::build_props(&self.tree, &id, props);
                self.write_content(&id, props, None, *funnel).await
            }
            Op::Retype { target } => {
                let cands: Vec<String> = ids
                    .iter()
                    .filter(|i| Self::page_like(&self.tree.nodes[*i]))
                    .cloned()
                    .collect();
                let id = Self::pick(&cands, *target)?.clone();
                let to = if self.tree.nodes[&id].node_type == PAGE {
                    DOC
                } else {
                    PAGE
                };
                let props = self.tree.nodes[&id].props.clone();
                self.write_content(&id, props, Some(to), Funnel::Tx).await
            }
            Op::Volatile { target, props } => {
                let cands: Vec<String> = ids
                    .iter()
                    .filter(|i| !Self::page_like(&self.tree.nodes[*i]))
                    .cloned()
                    .collect();
                let id = Self::pick(&cands, *target)?.clone();
                let props = Self::build_props(&self.tree, &id, props);
                let out = self.write_content(&id, props, None, Funnel::Tx).await;
                self.writes.push(super::model::InPlaceWrite {
                    op: self.op_index,
                    branch: MAIN.to_string(),
                    id,
                });
                out
            }
            Op::Delete {
                target,
                cascade,
                tx,
            } => {
                let id = Self::pick(&ids, *target)?.clone();
                let leaf = self.tree.kids(&id).is_empty();
                if (*tx || !*cascade) && !leaf {
                    return None;
                }
                // The repository delete refuses a node that any node still
                // references (referential integrity, cascading or not). Which
                // subtree members it checks is its business; the model skips
                // every delete whose subtree is referenced at all.
                let doomed = self.tree.subtree(&id);
                let referenced = self.tree.nodes.values().any(|n| {
                    super::model::refs_in(&n.props)
                        .iter()
                        .any(|(target, _)| doomed.contains(target))
                });
                if !*tx && referenced {
                    return None;
                }
                let path = self.tree.path(&id);
                self.tree.remove_subtree(&id);
                let res = if *tx {
                    let ctx = self.env.tx(MAIN).await;
                    match ctx.delete_node(WS, &id).await {
                        Ok(()) => ctx.commit().await,
                        Err(e) => Err(e),
                    }
                } else {
                    let options = DeleteNodeOptions {
                        cascade: *cascade,
                        check_has_children: true,
                        operation_meta: None,
                    };
                    self.env
                        .storage
                        .nodes()
                        .delete(self.env.scope(MAIN), &id, options)
                        .await
                        .map(|_| ())
                };
                let desc = format!("delete {id} {path} (tx={tx}, cascade={cascade})");
                match res {
                    Ok(()) => Some(desc),
                    Err(e) => self.fail(&desc, e),
                }
            }
            Op::Move { target, parent, tx } => {
                let id = Self::pick(&ids, *target)?.clone();
                let to = self.parent_sel(*parent);
                let n = &self.tree.nodes[&id];
                if to == n.parent
                    || (to != ROOT && self.tree.is_in_subtree(&id, &to))
                    || self.tree.name_taken(&to, &n.name)
                {
                    return None;
                }
                let from = self.tree.path(&id);
                self.tree.move_to(&id, &to);
                self.tree.nodes.get_mut(&id).unwrap().updated = seq;
                let new_path = self.tree.path(&id);
                let res = if *tx {
                    let ctx = self.env.tx(MAIN).await;
                    match ctx.move_node_tree(WS, &id, &new_path).await {
                        Ok(()) => ctx.commit().await,
                        Err(e) => Err(e),
                    }
                } else {
                    self.env
                        .storage
                        .nodes()
                        .move_node_tree(self.env.scope(MAIN), &id, &new_path, None)
                        .await
                };
                let desc = format!("move {id} {from} -> {new_path} (tx={tx})");
                match res {
                    Ok(()) => Some(desc),
                    Err(e) => self.fail(&desc, e),
                }
            }
            Op::Rename { target, name } => {
                let id = Self::pick(&ids, *target)?.clone();
                let name = name_of(*name);
                let parent = self.tree.nodes[&id].parent.clone();
                if self.tree.name_taken(&parent, &name) {
                    return None;
                }
                let from = self.tree.path(&id);
                self.tree.nodes.get_mut(&id).unwrap().name = name.clone();
                self.tree.nodes.get_mut(&id).unwrap().updated = seq;
                // A child's `parent` field is its parent's NAME, so renaming a
                // node rewrites its direct children's records too.
                for c in self.tree.kids(&id).to_vec() {
                    self.tree.nodes.get_mut(&c).unwrap().updated = seq;
                }
                let res = self
                    .env
                    .storage
                    .nodes()
                    .rename_node(self.env.scope(MAIN), &from, &name)
                    .await;
                let desc = format!("rename {id} {from} -> {name}");
                match res {
                    Ok(()) => Some(desc),
                    Err(e) => self.fail(&desc, e),
                }
            }
            Op::Reorder {
                target,
                anchor,
                before,
            } => {
                let id = Self::pick(&ids, *target)?.clone();
                let parent = self.tree.nodes[&id].parent.clone();
                let sibs: Vec<String> = self
                    .tree
                    .kids(&parent)
                    .iter()
                    .filter(|c| **c != id)
                    .cloned()
                    .collect();
                let anchor = Self::pick(&sibs, *anchor)?.clone();
                let order_before = self.tree.kids(&parent).to_vec();
                self.tree.reorder(&id, &anchor, *before);
                if self.tree.kids(&parent) == order_before.as_slice() {
                    // Already there: the system treats it as a no-op (no
                    // revision, no re-stamp), and so does the model.
                    self.tree.reorder(&id, &anchor, *before);
                    return None;
                }
                self.tree.nodes.get_mut(&id).unwrap().updated = seq;
                let parent_path = if parent == ROOT {
                    ROOT.to_string()
                } else {
                    self.tree.path(&parent)
                };
                let (name, anchor_name) = (
                    self.tree.nodes[&id].name.clone(),
                    self.tree.nodes[&anchor].name.clone(),
                );
                let ctx = self.env.tx(MAIN).await;
                let res = if *before {
                    ctx.reorder_child_before(WS, &parent_path, &name, &anchor_name)
                        .await
                } else {
                    ctx.reorder_child_after(WS, &parent_path, &name, &anchor_name)
                        .await
                };
                let res = match res {
                    Ok(()) => ctx.commit().await,
                    Err(e) => Err(e),
                };
                let side = if *before { "before" } else { "after" };
                let desc = format!("reorder {id} {side} {anchor} under {parent_path}");
                match res {
                    Ok(()) => Some(desc),
                    Err(e) => self.fail(&desc, e),
                }
            }
            Op::Copy {
                target,
                parent,
                name,
            } => self.copy_tree(*target, *parent, *name).await,
            Op::Translate {
                target,
                title,
                hide,
            } => {
                let id = Self::pick(&ids, *target)?.clone();
                let (overlay, model) = if *hide {
                    (LocaleOverlay::Hidden, Overlay::Hidden)
                } else {
                    let t = format!("{}-fr", TITLES[*title as usize % TITLES.len()]);
                    // `/__node_name` too: the node's translated French name,
                    // its French URL segment (plan Phase 12;
                    // `checks/localized.rs`).
                    let data = HashMap::from([
                        (
                            JsonPointer::new("/title"),
                            raisin_models::nodes::properties::PropertyValue::String(t.clone()),
                        ),
                        (
                            JsonPointer::new("/__node_name"),
                            raisin_models::nodes::properties::PropertyValue::String(t.clone()),
                        ),
                    ]);
                    (LocaleOverlay::Properties { data }, Overlay::Title(t))
                };
                self.tree.nodes.get_mut(&id).unwrap().overlay = Some(model.clone());
                self.overlay_events.healed.push((self.op_index, id.clone()));
                let ctx = self.env.tx(MAIN).await;
                let res = match ctx.store_translation(WS, &id, LOCALE, overlay).await {
                    Ok(()) => ctx.commit().await,
                    Err(e) => Err(e),
                };
                let desc = format!("translate {id} {model:?}");
                match res {
                    Ok(()) => Some(desc),
                    Err(e) => self.fail(&desc, e),
                }
            }
            Op::Restore { target, back } => self.restore(*target, *back).await,
            Op::Tx(steps) => self.apply_tx(steps).await,
            Op::ForkMerge(_) => unreachable!("handled by fork_merge"),
        }
    }
}
