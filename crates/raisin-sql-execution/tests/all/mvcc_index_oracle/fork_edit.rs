//! One edit on one side of a fork (`merge.rs` drives the fork itself).

use super::driver::Run;
use super::env::{MAIN, PAGE, WS};
use super::merge_model::Side;
use super::model::{InPlaceWrite, MNode, Snapshot, Tree, ROOT};
use super::ops::{name_of, Edit};

impl Run {
    /// One edit on `branch`, confined so the other side's structure is untouched.
    pub(super) async fn fork_edit(
        &mut self,
        branch: &str,
        tree: &mut Tree,
        me: &mut Side,
        other: &Side,
        base: &Tree,
        e: &Edit,
    ) {
        let ids: Vec<String> = tree
            .ids()
            .into_iter()
            .filter(|i| Self::page_like(&tree.nodes[i]))
            .collect();
        let ctx = self.env.tx(branch).await;
        let mut desc = format!("{e:?}");
        let mut in_place: Option<String> = None;
        let res = match e {
            Edit::Volatile { target, props } => {
                // MAIN only, on a node the feature side cannot touch (its
                // edits pick versioned nodes): an in-place write rewrites a
                // key the fork shares with `main` — exactly the key a merge
                // used to copy the feature's stale version back over.
                if branch != MAIN {
                    return;
                }
                let cands: Vec<String> = tree
                    .ids()
                    .into_iter()
                    .filter(|i| !Self::page_like(&tree.nodes[i]))
                    .filter(|i| !other.changed.contains(&tree.nodes[i].parent))
                    .collect();
                let Some(id) = Self::pick(&cands, *target).cloned() else {
                    return;
                };
                let mut props = Self::build_props(tree, &id, props);
                Self::refresh_refs(tree, &mut props);
                let n = tree.nodes.get_mut(&id).unwrap();
                n.props = props.clone();
                n.updated = self.seq;
                in_place = Some(id.clone());
                desc = format!("volatile {id}");
                match ctx.get_node(WS, &id).await {
                    Ok(Some(mut current)) => {
                        current.properties = props;
                        ctx.put_node(WS, &current).await
                    }
                    other => Err(raisin_error::Error::Backend(format!("tx read: {other:?}"))),
                }
            }
            Edit::Update { target, props } => {
                // Not under a parent the other side deleted: a resolution that
                // kept this node would resurrect it into a deleted parent.
                let cands: Vec<String> = ids
                    .iter()
                    .filter(|i| !other.changed.contains(&tree.nodes[*i].parent))
                    .cloned()
                    .collect();
                let Some(id) = Self::pick(&cands, *target).cloned() else {
                    return;
                };
                let props = Self::build_props(tree, &id, props);
                let n = tree.nodes.get_mut(&id).unwrap();
                n.props = props.clone();
                n.updated = self.seq;
                me.changed.insert(id.clone());
                desc = format!("update {id}");
                match ctx.get_node(WS, &id).await {
                    Ok(Some(mut current)) => {
                        current.properties = props;
                        ctx.put_node(WS, &current).await
                    }
                    other => Err(raisin_error::Error::Backend(format!("tx read: {other:?}"))),
                }
            }
            Edit::Create {
                parent,
                name,
                props,
            } => {
                let parent = match parent {
                    None => ROOT.to_string(),
                    Some(s) => match Self::pick(&ids, *s) {
                        Some(p) => p.clone(),
                        None => ROOT.to_string(),
                    },
                };
                let name = name_of(*name);
                if other.structural.contains(&parent)
                    || other.changed.contains(&parent)
                    || tree.name_taken(&parent, &name)
                    || (base.nodes.contains_key(&parent) || parent == ROOT)
                        && base.name_taken(&parent, &name)
                {
                    return;
                }
                let id = self.fresh_id();
                let props = Self::build_props(tree, &id, props);
                tree.append(MNode {
                    id: id.clone(),
                    name,
                    parent: parent.clone(),
                    node_type: PAGE.to_string(),
                    props,
                    created: self.seq,
                    updated: self.seq,
                    overlay: None,
                });
                me.changed.insert(id.clone());
                me.created.insert(id.clone());
                me.structural.insert(parent);
                desc = format!("create {id} at {}", tree.path(&id));
                ctx.add_node(WS, &Self::wire_node(tree, &id)).await
            }
            Edit::DeleteLeaf { target } => {
                let leaves: Vec<String> = ids
                    .iter()
                    .filter(|i| tree.kids(i).is_empty())
                    .filter(|i| !other.structural.contains(*i))
                    .filter(|i| !other.structural.contains(&tree.nodes[*i].parent))
                    // Nothing the other side changed may sit beneath it in the
                    // base (a resolution keeping that node would orphan it).
                    .filter(|i| {
                        !base.nodes.contains_key(*i)
                            || !base.subtree(i).iter().any(|d| other.changed.contains(d))
                    })
                    .cloned()
                    .collect();
                let Some(id) = Self::pick(&leaves, *target).cloned() else {
                    return;
                };
                me.structural.insert(tree.nodes[&id].parent.clone());
                me.changed.insert(id.clone());
                desc = format!("delete leaf {id} {}", tree.path(&id));
                tree.remove_subtree(&id);
                ctx.delete_node(WS, &id).await
            }
        };
        let res = match res {
            Ok(()) => ctx.commit().await,
            Err(e) => Err(e),
        };
        self.log.push(format!("    {branch}: {desc}"));
        if let Err(err) = res {
            self.anomalies.push(format!(
                "op #{} fork edit on {branch} {e:?} failed: {err}",
                self.op_index
            ));
        }
        let last = self
            .snaps
            .iter()
            .rev()
            .find(|s| s.branch == MAIN)
            .map(|s| s.head);
        if let (Some(id), true) = (in_place, branch == MAIN) {
            // Mints nothing: the newest `main` snapshot now reads the rewrite
            // (as `Run::record` does), and every earlier one — before the fork
            // via the in-place write log, inside the window right here — no
            // longer holds what it held.
            let newest = self
                .snaps
                .iter()
                .rposition(|s| s.branch == MAIN)
                .expect("initial snapshot");
            for s in self.snaps[..newest].iter_mut().filter(|s| s.branch == MAIN) {
                if s.op == self.op_index && s.tree.nodes.contains_key(&id) {
                    s.tainted.insert(id.clone());
                }
            }
            self.snaps[newest].tree = tree.clone();
            self.snaps[newest].op = self.op_index;
            self.writes.push(InPlaceWrite {
                op: self.op_index,
                branch: MAIN.to_string(),
                id,
            });
            return;
        }
        if branch == MAIN {
            let head = self.env.head(MAIN).await;
            if Some(head) == last {
                self.anomalies.push(format!(
                    "op #{} fork edit on main minted no revision",
                    self.op_index
                ));
                return;
            }
            self.snaps.push(Snapshot {
                branch: MAIN.to_string(),
                head,
                op: self.op_index,
                tree: tree.clone(),
                tainted: Default::default(),
                retro: false,
                overlay_gap: Default::default(),
                replica: false,
            });
        }
    }
}
