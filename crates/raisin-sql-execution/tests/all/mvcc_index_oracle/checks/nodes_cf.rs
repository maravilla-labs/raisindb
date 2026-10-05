//! `cf::NODES`, decoded straight off `storage.db()`, against the model — and
//! the `Node.order_key == ORDERED_CHILDREN label` invariant.

use super::{name, Checker};
use crate::mvcc_index_oracle::env::{REPO, TENANT, WS};
use crate::mvcc_index_oracle::model::{Snapshot, ROOT};
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage};
use std::collections::{BTreeMap, HashMap};

/// The newest version at or before `head` of every node id in the workspace:
/// `None` for a tombstone.
pub fn nodes_at(checker: &Checker<'_>, branch: &str, head: &HLC) -> BTreeMap<String, Option<Node>> {
    let db = checker.env.storage.db();
    let cf = db.cf_handle(raisin_rocksdb::cf::NODES).expect("nodes cf");
    let prefix = format!("{TENANT}\0{REPO}\0{branch}\0{WS}\0nodes\0").into_bytes();
    let mut out: BTreeMap<String, Option<Node>> = BTreeMap::new();
    for (key, value) in db
        .prefix_iterator_cf(&cf, &prefix)
        .flatten()
        .take_while(|(k, _)| k.starts_with(&prefix))
    {
        let rest = &key[prefix.len()..];
        if rest.len() < 18 {
            continue;
        }
        let (middle, rev) = rest.split_at(rest.len() - 16);
        // `{id}\0`; anything with a second separator (adjacency rows) is not
        // a version of the node record.
        let Some(id) = middle.strip_suffix(b"\0") else {
            continue;
        };
        if id.contains(&0) {
            continue;
        }
        let id = String::from_utf8_lossy(id).to_string();
        let Ok(rev) = HLC::decode_descending(rev) else {
            continue;
        };
        if rev > *head || out.contains_key(&id) {
            // Keys sort newest first within an id: the first one <= head wins.
            continue;
        }
        if raisin_rocksdb::keys::is_tombstone_value(&value) {
            out.insert(id, None);
            continue;
        }
        match raisin_rocksdb::decode_node_blob(&value) {
            Ok((node, _parent_id)) => {
                out.insert(id, Some(node));
            }
            Err(_) => {
                out.insert(id, None);
            }
        }
    }
    out
}

impl Checker<'_> {
    pub async fn nodes_cf(&mut self, s: &Snapshot) {
        let stored = nodes_at(self, &s.branch, &s.head);
        let live: Vec<String> = stored
            .iter()
            .filter(|(_, v)| v.is_some())
            .map(|(k, _)| k.clone())
            .collect();
        let model: Vec<String> = s.tree.ids();
        self.expect_eq(name::NODES_CF, s, "live ids", live, model);
        // No funnel — transaction, repository, merge, replication — writes a
        // legacy full-`Node` blob any more: none of them embeds a path.
        for (id, n) in stored.iter().filter_map(|(id, n)| Some((id, n.as_ref()?))) {
            if !n.path.is_empty() {
                self.report(
                    name::RECORD_FORMAT,
                    s,
                    format!("{id}'s blob embeds the path {:?} (legacy format)", n.path),
                );
            }
        }
        let mut labels: HashMap<String, String> = HashMap::new();
        let mut parents: Vec<String> = s.tree.children.keys().cloned().collect();
        parents.sort();
        for p in parents {
            let page = self
                .env
                .storage
                .nodes()
                .list_ordered_children_page(
                    self.env.scope(&s.branch),
                    &p,
                    None,
                    None,
                    false,
                    Some(&s.head),
                )
                .await;
            if let Ok(page) = page {
                for c in page {
                    labels.insert(c.child_id, c.order_label);
                }
            }
        }
        for (id, m) in &s.tree.nodes {
            let Some(Some(n)) = stored.get(id) else {
                continue;
            };
            let parent_name = if m.parent == ROOT {
                ROOT.to_string()
            } else {
                s.tree.nodes[&m.parent].name.clone()
            };
            self.expect_eq(
                name::NODES_CF,
                s,
                &format!("{id} identity (name, type, parent)"),
                (n.name.clone(), n.node_type.clone(), n.parent.clone()),
                (m.name.clone(), m.node_type.clone(), Some(parent_name)),
            );
            if !s.tainted.contains(id) {
                // `$`-prefixed keys are engine-stamped membership
                // (`$mixins`, `$supertypes`), not content the op log wrote.
                let stored: std::collections::HashMap<_, _> = n
                    .properties
                    .iter()
                    .filter(|(k, _)| !k.starts_with('$'))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                self.expect_eq(
                    name::NODES_CF,
                    s,
                    &format!("{id} properties"),
                    &stored,
                    &m.props,
                );
            }
            match labels.get(id) {
                Some(label) => self.expect_eq(
                    name::ORDER_KEY_LABEL,
                    s,
                    &format!("{id} order_key vs ORDERED_CHILDREN label"),
                    &n.order_key,
                    label,
                ),
                None => self.report(
                    name::ORDER_KEY_LABEL,
                    s,
                    format!("{id} has no live ORDERED_CHILDREN entry under {}", m.parent),
                ),
            }
        }
    }
}
