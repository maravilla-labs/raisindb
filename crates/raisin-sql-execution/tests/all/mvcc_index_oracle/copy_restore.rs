//! Copy-tree and RESTORE: the two ops whose result depends on more than the
//! op itself (minted ids; a historical version).

use super::driver::Run;
use super::env::{query, MAIN, WS};
use super::model::{MNode, ROOT};
use super::ops::name_of;
use raisin_storage::{NodeRepository, Storage};

impl Run {
    /// Copy `target`'s subtree under `parent` as `name`.
    ///
    /// The copy MINTS ids the op log cannot know. Those ids are the one thing
    /// the model takes from the system: the root's from the copy's return value,
    /// each descendant's from `get_node_id_by_path` of the path the MODEL
    /// predicts, read once at the copy's revision. Everything else about the
    /// copies — names, order, content, timestamps — stays model-derived, and a
    /// predicted path the system cannot resolve is an anomaly, not a skip.
    pub async fn copy_tree(
        &mut self,
        target: u16,
        parent: Option<u16>,
        name: u8,
    ) -> Option<String> {
        let ids = self.scoped_ids();
        let src = Self::pick(&ids, target)?.clone();
        let to = self.parent_sel(parent);
        let name = name_of(name);
        if (to != ROOT && self.tree.is_in_subtree(&src, &to)) || self.tree.name_taken(&to, &name) {
            return None;
        }
        let src_path = self.tree.path(&src);
        let to_path = if to == ROOT {
            ROOT.to_string()
        } else {
            self.tree.path(&to)
        };
        let ctx = self.env.tx(MAIN).await;
        let res = ctx
            .copy_node_tree(WS, &src_path, &to_path, Some(&name), "oracle")
            .await;
        let res = match res {
            Ok(root) => ctx.commit().await.map(|_| root),
            Err(e) => Err(e),
        };
        let desc = format!("copy {src} {src_path} -> {to_path}/{name}");
        let root = match res {
            Ok(root) => root,
            Err(e) => return self.fail(&desc, e),
        };

        // Model: the copies, in the source's document order.
        let order = self.tree.subtree(&src);
        let mut map = std::collections::HashMap::new();
        map.insert(src.clone(), root.id.clone());
        let mut planned = Vec::new();
        for old in &order {
            let o = self.tree.nodes[old].clone();
            let (new_parent, new_name) = if *old == src {
                (to.clone(), name.clone())
            } else {
                (map[&o.parent].clone(), o.name.clone())
            };
            let new_id = if *old == src {
                root.id.clone()
            } else {
                // Predicted path: the parent's copy path plus the name.
                let parent_path = planned
                    .iter()
                    .find(|(id, _): &&(String, String)| *id == new_parent)
                    .map(|(_, p)| p.clone())
                    .expect("parent planned first");
                let path = format!("{parent_path}/{new_name}");
                match self
                    .env
                    .storage
                    .nodes()
                    .get_node_id_by_path(self.env.scope(MAIN), &path, None)
                    .await
                {
                    Ok(Some(found)) => found,
                    other => {
                        self.anomalies.push(format!(
                            "op #{} copy: predicted copy path {path} does not resolve ({other:?})",
                            self.op_index
                        ));
                        return Some(format!("{desc} (UNBOUND)"));
                    }
                }
            };
            let path = if *old == src {
                format!("{}/{}", to_path.trim_end_matches('/'), name)
            } else {
                let pp = &planned
                    .iter()
                    .find(|(id, _): &&(String, String)| *id == new_parent)
                    .unwrap()
                    .1;
                format!("{pp}/{new_name}")
            };
            planned.push((new_id.clone(), path));
            map.insert(old.clone(), new_id.clone());
            self.tree.append(MNode {
                id: new_id,
                name: new_name,
                parent: new_parent,
                created: self.seq,
                updated: self.seq,
                ..o
            });
        }
        Some(format!("{desc} as {}", root.id))
    }

    /// `RESTORE NODE id=... TO REVISION <snapshot head>`.
    ///
    /// Any snapshot of the node — including one from before it (or an
    /// ancestor) was moved or renamed: RESTORE reads the historical node BY ID
    /// and takes its content, while its path, name, parent and label stay
    /// current.
    pub async fn restore(&mut self, target: u16, back: u16) -> Option<String> {
        let cands: Vec<String> = self
            .scoped_ids()
            .into_iter()
            .filter(|i| Self::page_like(&self.tree.nodes[i]))
            .collect();
        let id = Self::pick(&cands, target)?.clone();
        let cur = self.tree.nodes[&id].clone();
        let options: Vec<usize> = self
            .snaps
            .iter()
            .enumerate()
            // Never into a merged fork's window: those revisions were rewritten
            // by the merge (expected failure `main_revision_inside_merged_fork`),
            // so what RESTORE reads there is not what the model recorded.
            .filter(|(_, s)| s.branch == MAIN && !s.retro)
            .filter(|(_, s)| match s.tree.get(&id) {
                // Same type too: RESTORE goes through `update_node`, which
                // refuses a node_type change.
                Some(h) => h.node_type == cur.node_type && h.props != cur.props,
                None => false,
            })
            .map(|(i, _)| i)
            .collect();
        let snap = &self.snaps[*Self::pick_idx(&options, back)?];
        let hist = snap.tree.nodes[&id].clone();
        let rev = snap.head;
        let mut restored = hist.props.clone();
        Self::refresh_refs(&self.tree, &mut restored);
        {
            let n = self.tree.nodes.get_mut(&id).unwrap();
            n.props = restored;
            n.node_type = hist.node_type.clone();
            n.updated = self.seq;
        }
        let sql = format!(
            "RESTORE NODE id='{id}' TO REVISION {}",
            Self::restore_rev(&rev)
        );
        let desc = format!("restore {id} to {rev}");
        match query(&self.env.engine(MAIN), &sql).await {
            Ok(_) => Some(desc),
            Err(e) => self.fail(&desc, e),
        }
    }

    fn pick_idx(cands: &[usize], sel: u16) -> Option<&usize> {
        if cands.is_empty() {
            None
        } else {
            Some(&cands[sel as usize % cands.len()])
        }
    }
}
