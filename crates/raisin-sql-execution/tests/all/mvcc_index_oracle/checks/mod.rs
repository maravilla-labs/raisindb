//! Templates: each asks the system one index-backed question at one recorded
//! revision and compares the answer with the model's.

mod batch_get;
mod content;
mod localized;
mod nodes_cf;
mod order;
mod tree;
mod tree_sql;

use super::env::{query, Env, Store};
use super::model::Snapshot;
use raisin_sql_execution::QueryEngine;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// Template names. The expected-failure list refers to these.
pub mod name {
    pub const NODES_CF: &str = "nodes_cf";
    /// `get_many_for_read` equals one `get`/`get_by_path` per item (plan
    /// Phase 4).
    pub const BATCH_GET: &str = "batch_get_equals_get";
    /// Every stored blob is the one record format (plan Phase 10b): a
    /// `StorageNode`, no embedded path — whichever funnel wrote it.
    pub const RECORD_FORMAT: &str = "one_record_format";
    pub const ORDER_KEY_LABEL: &str = "order_key_equals_label";
    pub const GET_BY_PATH: &str = "get_by_path";
    pub const LIST_BY_PARENT: &str = "list_by_parent+has_children";
    pub const CHILD_OF_ORDER: &str = "child_of_order_by___order";
    pub const DESCENDANT_PAGES: &str = "descendant_of_tree_order_pages";
    pub const PROPERTY_EQ: &str = "property_eq";
    pub const NODE_TYPE_EQ: &str = "node_type_eq";
    pub const REFERENCES: &str = "references";
    pub const COMPOUND: &str = "compound_child_of_order_by_created_at";
    pub const COMPOUND_HISTORICAL: &str = "compound_child_of_order_by_created_at_past_revision";
    pub const TS_ORDER: &str = "order_by_timestamp_limit";
    pub const TS_RANGE: &str = "timestamp_range";
    pub const COUNT: &str = "count";
    pub const RESOLVE: &str = "resolve_depth_2";
    pub const TRANSLATION: &str = "locale_read";
    pub const TRANSLATION_HISTORICAL: &str = "locale_read_at_past_revision";
    /// A locale read on a stage-3 replica.
    pub const TRANSLATION_REPLICA: &str = "locale_read_on_replica";
    /// A locale read of a node a merge resolution kept over a delete.
    pub const TRANSLATION_AFTER_RESOLUTION: &str = "locale_read_after_resolution_over_delete";
    pub const LAST_LABEL: &str = "next_append_label";
    /// `RESOLVE_PATH` over the localized name index (plan Phase 12).
    pub const LOCALIZED_PATH: &str = "localized_path_lookup";
    /// Any template, at a `main` revision taken between a fork and its merge.
    pub const MERGE_RETRO: &str = "main_revision_inside_merged_fork";
}

#[derive(Clone, Debug)]
pub struct Mismatch {
    pub template: &'static str,
    pub branch: String,
    pub head: String,
    /// Index of the op after which the snapshot was taken.
    pub op: usize,
    pub detail: String,
}

pub struct Checker<'a> {
    pub env: &'a Env,
    engines: HashMap<String, QueryEngine<Store>>,
    pub instants: &'a [(u64, String)],
    pub out: Vec<Mismatch>,
    /// Whether this snapshot is the newest one of its branch (HEAD-only
    /// templates run there).
    pub at_head: bool,
    /// Every path each branch's model ever held: a path no longer live must
    /// resolve to nothing.
    pub seen_paths: HashMap<String, BTreeSet<String>>,
    /// Ask the SQL templates in their HEAD form (no `__revision`): the planner
    /// picks different readers then (ORDERED_CHILDREN, the compound index), so
    /// the newest snapshot of a branch is asked both ways.
    pub head_form: bool,
}

impl<'a> Checker<'a> {
    pub fn new(env: &'a Env, instants: &'a [(u64, String)]) -> Self {
        Self {
            env,
            engines: HashMap::new(),
            instants,
            out: Vec::new(),
            at_head: false,
            seen_paths: HashMap::new(),
            head_form: false,
        }
    }

    pub fn engine(&mut self, branch: &str) -> &QueryEngine<Store> {
        if !self.engines.contains_key(branch) {
            let e = self.env.engine(branch);
            self.engines.insert(branch.to_string(), e);
        }
        &self.engines[branch]
    }

    pub async fn sql(&mut self, s: &Snapshot, sql: &str) -> Result<Vec<Value>, String> {
        self.engine(&s.branch);
        query(&self.engines[&s.branch], sql).await
    }

    pub fn report(&mut self, template: &'static str, s: &Snapshot, detail: String) {
        let template = if s.retro { name::MERGE_RETRO } else { template };
        let detail = if self.head_form {
            format!("[HEAD form] {detail}")
        } else {
            detail
        };
        self.out.push(Mismatch {
            template,
            branch: s.branch.clone(),
            head: s.head.to_string(),
            op: s.op,
            detail,
        });
    }

    /// Compare and report.
    pub fn expect_eq<T: PartialEq + std::fmt::Debug>(
        &mut self,
        template: &'static str,
        s: &Snapshot,
        what: &str,
        actual: T,
        expected: T,
    ) {
        if actual != expected {
            self.report(
                template,
                s,
                format!("{what}: got {actual:?}, model {expected:?}"),
            );
        }
    }

    /// ` AND __revision = '…'`, or nothing in the HEAD form.
    pub fn at(&self, s: &Snapshot) -> String {
        if self.head_form {
            String::new()
        } else {
            format!(" AND __revision = '{}'", s.head)
        }
    }

    /// ` WHERE __revision = '…'`, or nothing in the HEAD form.
    pub fn where_rev(&self, s: &Snapshot) -> String {
        if self.head_form {
            String::new()
        } else {
            format!(" WHERE __revision = '{}'", s.head)
        }
    }

    /// Run every template against one snapshot; the newest snapshot of a
    /// branch is asked a second time in the HEAD form.
    pub async fn check(&mut self, s: &Snapshot, at_head: bool) {
        self.at_head = at_head;
        self.head_form = false;
        self.engine(&s.branch);
        self.nodes_cf(s).await;
        self.batch_get(s).await;
        self.tree_templates(s).await;
        self.order_templates(s).await;
        self.content_templates(s).await;
        self.localized_templates(s).await;
        if at_head {
            self.head_form = true;
            self.sql_tree_templates(s).await;
            self.order_templates(s).await;
            self.content_templates(s).await;
            self.localized_templates(s).await;
            self.head_form = false;
        }
    }
}

/// Drop tainted ids from a list (both sides of a content comparison).
pub fn untainted(s: &Snapshot, ids: Vec<String>) -> Vec<String> {
    ids.into_iter().filter(|i| !s.tainted.contains(i)).collect()
}

pub fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// One SQL of every index-backed template shape, for plan assertions.
/// `at` is ` AND __revision = '…'` (or empty for the HEAD form).
pub fn template_shapes(at: &str) -> Vec<String> {
    use crate::mvcc_index_oracle::env::{PAGE, WS};
    let only = at.replacen(" AND ", " WHERE ", 1);
    vec![
        format!("SELECT id FROM '{WS}' WHERE properties->>'title'::String = 'alpha'{at}"),
        format!("SELECT id FROM '{WS}' WHERE node_type = '{PAGE}'{at}"),
        format!("SELECT id FROM '{WS}' WHERE REFERENCES('{WS}:/n0'){at}"),
        format!("SELECT id, path FROM '{WS}' WHERE CHILD_OF('/n0'){at} ORDER BY __order"),
        format!("SELECT id, path, __tree_order FROM '{WS}' WHERE DESCENDANT_OF('/n0'){at} ORDER BY __tree_order LIMIT 2"),
        format!("SELECT id FROM '{WS}' WHERE CHILD_OF('/n0'){at} ORDER BY created_at DESC LIMIT 3"),
        format!("SELECT id FROM '{WS}' WHERE CHILD_OF('/n0') AND node_type = '{PAGE}'{at} ORDER BY created_at DESC LIMIT 3"),
        format!("SELECT id FROM '{WS}' WHERE node_type = '{PAGE}' AND properties->>'__parent_path'::String = '/n0'{at} ORDER BY created_at DESC LIMIT 3"),
        format!("SELECT id FROM '{WS}'{only} ORDER BY updated_at DESC LIMIT 4"),
        format!("SELECT id FROM '{WS}' WHERE updated_at >= '2020-01-01T00:00:00Z'::TIMESTAMPTZ AND updated_at < '2100-01-01T00:00:00Z'::TIMESTAMPTZ{at}"),
        format!("SELECT COUNT(*) FROM '{WS}'{only}"),
        format!("SELECT COUNT(*) FROM '{WS}' WHERE properties->>'title'::String = 'alpha'{at}"),
    ]
}
