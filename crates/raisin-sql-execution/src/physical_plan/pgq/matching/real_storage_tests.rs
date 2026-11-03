//! The pushdown against a REAL RocksDB, with the relations each access path
//! returns counted.
//!
//! `pushdown_tests` proves the logic on an in-memory graph. This module proves
//! that the per-node indexes of the real store return what the global index
//! holds (revisions, tombstones, packed adjacency) and carries the benchmark
//! numbers the book quotes: run it with
//!
//! ```text
//! cargo test -p raisin-sql-execution --lib --release real_storage_tests::bench \
//!     -- --ignored --nocapture
//! ```

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::{Node, RelationRef};
use raisin_sql::ast::pgq_parser::parse_graph_table;
use raisin_sql::ast::{PathPattern, PatternElement};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RelationRepository, RepoScope, Storage,
    StorageScope, WorkspaceRepository,
};
use tempfile::TempDir;

use super::path_semantics_for;
use super::seek::SeekHints;
use super::single_hop::match_single_hop_with;
use super::variable_length::execute_variable_length_with;
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::graph_access::Side;
use crate::physical_plan::graph_access::{RelationAccess, ScannedRelation, StorageAccess};
use crate::physical_plan::graph_budget::GraphBudget;
use crate::physical_plan::graph_neighborhood::{
    neighborhood, NeighborhoodParams, NodeGate, DEFAULT_MAX_NODES, MAX_VISITED, TOO_LARGE, TOO_SLOW,
};
use crate::physical_plan::pgq::types::VariableBinding;
use raisin_models::nodes::RelationRef as Ref;

const TENANT: &str = "t_real";
const REPO: &str = "r_real";
const BRANCH: &str = "main";
const WS: &str = "social";
const TYPE: &str = "test:Person";

type Store = Arc<raisin_rocksdb::RocksDBStorage>;

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

async fn storage() -> (Store, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = raisin_rocksdb::RocksDBStorage::new(tmp.path()).unwrap();
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "t", None, None, false, false)
        .await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .unwrap();
    (Arc::new(storage), tmp)
}

async fn node(storage: &Store, id: &str) {
    storage
        .nodes()
        .create(
            scope(),
            Node {
                id: id.to_string(),
                path: format!("/{id}"),
                name: id.to_string(),
                parent: Some("/".to_string()),
                node_type: TYPE.to_string(),
                properties: HashMap::<String, PropertyValue>::new(),
                ..Default::default()
            },
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

async fn link(storage: &Store, from: &str, to: &str, ty: &str) {
    storage
        .relations()
        .add_relation(
            scope(),
            from,
            TYPE,
            RelationRef::new(to.into(), WS.into(), TYPE.into(), ty.into(), None),
        )
        .await
        .unwrap();
}

/// Counts what each access path hands back.
struct Counting<A> {
    inner: A,
    global: AtomicUsize,
    per_node: AtomicUsize,
}

impl<A> Counting<A> {
    fn new(inner: A) -> Self {
        Self {
            inner,
            global: AtomicUsize::new(0),
            per_node: AtomicUsize::new(0),
        }
    }
    fn reset(&self) {
        self.global.store(0, Ordering::SeqCst);
        self.per_node.store(0, Ordering::SeqCst);
    }
    fn counts(&self) -> (usize, usize) {
        (
            self.global.load(Ordering::SeqCst),
            self.per_node.load(Ordering::SeqCst),
        )
    }
}

impl<A: RelationAccess> RelationAccess for Counting<A> {
    async fn global_scan(
        &self,
        relation_type: Option<&str>,
    ) -> Result<Vec<ScannedRelation>, ExecutionError> {
        let r = self.inner.global_scan(relation_type).await?;
        self.global.fetch_add(r.len(), Ordering::SeqCst);
        Ok(r)
    }
    fn workspaces(&self) -> impl Future<Output = Result<Vec<String>, ExecutionError>> + Send {
        self.inner.workspaces()
    }
    fn node_type(
        &self,
        w: &str,
        id: &str,
    ) -> impl Future<Output = Result<Option<String>, ExecutionError>> + Send {
        self.inner.node_type(w, id)
    }
    fn node_by_path(
        &self,
        w: &str,
        p: &str,
    ) -> impl Future<Output = Result<Option<(String, String)>, ExecutionError>> + Send {
        self.inner.node_by_path(w, p)
    }
    fn readable(
        &self,
        w: &str,
        id: &str,
    ) -> impl Future<Output = Result<bool, ExecutionError>> + Send {
        self.inner.readable(w, id)
    }
    async fn outgoing(&self, w: &str, id: &str) -> Result<Vec<Ref>, ExecutionError> {
        let r = self.inner.outgoing(w, id).await?;
        self.per_node.fetch_add(r.len(), Ordering::SeqCst);
        Ok(r)
    }
    async fn incoming(
        &self,
        w: &str,
        id: &str,
    ) -> Result<Vec<(String, String, Ref)>, ExecutionError> {
        let r = self.inner.incoming(w, id).await?;
        self.per_node.fetch_add(r.len(), Ordering::SeqCst);
        Ok(r)
    }
}

struct Open;
impl NodeGate for Open {
    type Item = ();
    async fn report(&self, _: &str, _: &str, _: u32) -> Result<Option<()>, ExecutionError> {
        Ok(Some(()))
    }
}

struct Parsed {
    pattern: PathPattern,
    hints: SeekHints,
}

fn parse(match_sql: &str, where_sql: &str) -> Parsed {
    let q = parse_graph_table(&format!(
        "GRAPH_TABLE(MATCH {match_sql} WHERE {where_sql} COLUMNS (a.id AS x))"
    ))
    .unwrap();
    Parsed {
        pattern: q.match_clause.patterns[0].clone(),
        hints: SeekHints::from_where(q.where_clause.as_ref().map(|w| &w.expression)),
    }
}

async fn hop<A: RelationAccess>(
    a: &A,
    p: &Parsed,
    hints: &SeekHints,
) -> BTreeSet<(String, String)> {
    let [PatternElement::Node(s), PatternElement::Relationship(r), PatternElement::Node(t)] =
        p.pattern.elements.as_slice()
    else {
        panic!("not a hop")
    };
    match_single_hop_with(a, s, r, t, hints)
        .await
        .unwrap()
        .iter()
        .map(|b| {
            (
                b.get_node("a").map(|n| n.id.clone()).unwrap_or_default(),
                b.get_node("b").map(|n| n.id.clone()).unwrap_or_default(),
            )
        })
        .collect()
}

async fn var_len<A: RelationAccess>(a: &A, p: &Parsed, hints: &SeekHints) -> usize {
    let [PatternElement::Node(s), PatternElement::Relationship(r), PatternElement::Node(t)] =
        p.pattern.elements.as_slice()
    else {
        panic!("not a hop")
    };
    execute_variable_length_with(
        a,
        s,
        r,
        t,
        &path_semantics_for(&p.pattern),
        vec![VariableBinding::new()],
        hints,
    )
    .await
    .unwrap()
    .len()
}

fn access(storage: &Store) -> Counting<StorageAccess<'_, raisin_rocksdb::RocksDBStorage>> {
    Counting::new(StorageAccess::new(storage, TENANT, REPO, BRANCH, None))
}

#[tokio::test]
async fn a_pinned_hop_reads_only_the_nodes_own_relations_in_a_real_store() {
    let (storage, _tmp) = storage().await;
    for i in 0..300 {
        node(&storage, &format!("r{i}")).await;
    }
    for i in 0..300 {
        link(
            &storage,
            &format!("r{i}"),
            &format!("r{}", (i + 1) % 300),
            "k",
        )
        .await;
    }
    for i in 10..15 {
        link(&storage, "r0", &format!("r{i}"), "k").await;
    }
    // a removed relation must not come back through either route
    storage
        .relations()
        .remove_relation(scope(), "r0", WS, "r12", Some("k"))
        .await
        .unwrap();

    let acc = access(&storage);
    let p = parse("(a)-[r:k]->(b)", "a.id = 'r0'");
    let global: BTreeSet<_> = hop(&acc, &p, &SeekHints::none())
        .await
        .into_iter()
        .filter(|x| x.0 == "r0")
        .collect();
    assert_eq!(
        acc.counts().0,
        300 + 5 - 1,
        "global route reads every relation"
    );
    acc.reset();

    let seeked = hop(&acc, &p, &p.hints).await;
    assert_eq!(
        acc.counts(),
        (0, 5),
        "r0 has 1 ring + 5 extra - 1 removed = 5"
    );
    assert_eq!(seeked, global);
    assert!(!seeked.contains(&("r0".into(), "r12".into())));

    // the same through the incoming index
    acc.reset();
    let p = parse("(a)-[r:k]->(b)", "b.id = 'r13'");
    let seeked = hop(&acc, &p, &p.hints).await;
    assert_eq!(acc.counts(), (0, 2), "r12->r13 and r0->r13");
    assert_eq!(seeked.len(), 2);
}

#[tokio::test]
async fn variable_length_and_neighborhood_in_a_real_store() {
    let (storage, _tmp) = storage().await;
    for i in 0..200 {
        node(&storage, &format!("r{i}")).await;
    }
    for i in 0..200 {
        link(
            &storage,
            &format!("r{i}"),
            &format!("r{}", (i + 1) % 200),
            "k",
        )
        .await;
    }
    let acc = access(&storage);
    let p = parse("(a)-[r:k]->{1,3}(b)", "a.id = 'r5'");
    assert_eq!(var_len(&acc, &p, &p.hints).await, 3);
    assert_eq!(acc.counts(), (0, 3), "r5, r6, r7 are expanded");

    acc.reset();
    let hits = neighborhood(
        &acc,
        &Open,
        &[(WS.to_string(), "r5".to_string())],
        &NeighborhoodParams {
            side: Side::Out,
            types: vec![],
            min_depth: 1,
            max_depth: 3,
            max_nodes: DEFAULT_MAX_NODES,
            max_visited: MAX_VISITED,
            budget: GraphBudget::standard(),
        },
    )
    .await
    .unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(acc.counts(), (0, 3));
}

/// A ring of `n` nodes `b0 -> b1 -> ...` plus a hub `b0` linked to all of them.
async fn hub_ring(storage: &Store, n: usize) {
    for i in 0..n {
        node(storage, &format!("b{i}")).await;
    }
    for i in 0..n {
        link(storage, &format!("b{i}"), &format!("b{}", (i + 1) % n), "k").await;
    }
    for i in 2..n {
        link(storage, "b0", &format!("b{i}"), "k").await;
    }
}

fn shown_without_digits(e: &ExecutionError, text: &str) {
    let shown = e.to_string();
    assert!(shown.contains(text), "{shown}");
    assert!(!shown.chars().any(|c| c.is_ascii_digit()), "{shown}");
}

#[tokio::test]
async fn neighborhood_limits_hold_in_a_real_store() {
    let (storage, _tmp) = storage().await;
    hub_ring(&storage, 60).await;
    let acc = access(&storage);
    let from = [(WS.to_string(), "b0".to_string())];
    let params = |budget: GraphBudget| NeighborhoodParams {
        side: Side::Out,
        types: vec![],
        min_depth: 1,
        max_depth: 3,
        max_nodes: DEFAULT_MAX_NODES,
        max_visited: MAX_VISITED,
        budget,
    };

    // Enough of everything: the whole ring.
    let all = neighborhood(&acc, &Open, &from, &params(GraphBudget::standard()))
        .await
        .unwrap();
    assert_eq!(all.len(), 59);
    let (_, read) = acc.counts();

    // One relation fewer than the search reads: refused, no counts shown.
    let err = neighborhood(
        &acc,
        &Open,
        &from,
        &params(GraphBudget::new(
            std::time::Duration::from_secs(60),
            read - 1,
        )),
    )
    .await
    .unwrap_err();
    shown_without_digits(&err, TOO_LARGE);

    // A deadline that has passed: refused before the first seek.
    acc.reset();
    let err = neighborhood(
        &acc,
        &Open,
        &from,
        &params(GraphBudget::until(Instant::now(), usize::MAX)),
    )
    .await
    .unwrap_err();
    shown_without_digits(&err, TOO_SLOW);
    assert_eq!(acc.counts(), (0, 0));
}

#[tokio::test]
async fn a_seeded_expansion_over_budget_takes_the_global_route_in_a_real_store() {
    use super::variable_length::execute_variable_length_budgeted;
    let (storage, _tmp) = storage().await;
    hub_ring(&storage, 60).await;
    let acc = access(&storage);
    for (pat, wh) in [
        ("(a)-[r:k]->{1,2}(b)", "a.id = 'b0'"),
        (
            "ANY SHORTEST p = (a)-[r:k]->{1,4}(b)",
            "a.id = 'b5' AND b.id = 'b8'",
        ),
    ] {
        let p = parse(pat, wh);
        let [PatternElement::Node(s), PatternElement::Relationship(r), PatternElement::Node(t)] =
            p.pattern.elements.as_slice()
        else {
            panic!("not a hop")
        };
        let semantics = path_semantics_for(&p.pattern);
        let run = |budget: GraphBudget| {
            execute_variable_length_budgeted(
                &acc,
                s,
                r,
                t,
                &semantics,
                vec![VariableBinding::new()],
                &p.hints,
                budget,
            )
        };
        let rows = |v: Vec<VariableBinding>| -> BTreeSet<(String, String)> {
            v.iter()
                .map(|b| {
                    (
                        b.get_node("a").map(|n| n.id.clone()).unwrap_or_default(),
                        b.get_node("b").map(|n| n.id.clone()).unwrap_or_default(),
                    )
                })
                .collect()
        };
        // What the WHERE clause keeps: rows whose pinned variables are the pins.
        let pin_of = |v: &str| {
            p.hints
                .seek_for(Some(&v.to_string()))
                .and_then(|s| s.ids.clone())
        };
        let (pa, pb) = (pin_of("a"), pin_of("b"));
        let keep = |set: BTreeSet<(String, String)>| -> BTreeSet<(String, String)> {
            set.into_iter()
                .filter(|x| pa.as_ref().is_none_or(|ids| ids.contains(&x.0)))
                .filter(|x| pb.as_ref().is_none_or(|ids| ids.contains(&x.1)))
                .collect()
        };

        acc.reset();
        let seeded = keep(rows(run(GraphBudget::standard()).await.unwrap()));
        assert_eq!(acc.counts().0, 0, "{pat}: within budget, no global scan");
        assert!(!seeded.is_empty(), "{pat}: the fixture should match");

        for budget in [
            GraphBudget::new(std::time::Duration::from_secs(60), 1),
            GraphBudget::until(Instant::now(), usize::MAX),
        ] {
            acc.reset();
            let fallback = keep(rows(run(budget).await.unwrap()));
            assert!(
                acc.counts().0 > 0,
                "{pat}: over budget, the global route ran"
            );
            assert_eq!(fallback, seeded, "{pat}: same answer either way");
        }
    }
}

/// Builds `nodes` nodes with about `relations` relations (a hub-free random
/// graph), then times the one-node queries both ways.
async fn bench_case(nodes: usize, relations: usize) {
    let (storage, _tmp) = storage().await;
    let t0 = Instant::now();
    for i in 0..nodes {
        node(&storage, &format!("n{i}")).await;
    }
    let mut state = 12345u64;
    let mut next = |m: usize| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as usize) % m
    };
    let mut made = 0;
    while made < relations {
        let (a, b) = (next(nodes), next(nodes));
        if a != b {
            link(&storage, &format!("n{a}"), &format!("n{b}"), "k").await;
            made += 1;
        }
    }
    println!(
        "\n== {nodes} nodes, {relations} relations (built in {:?}) ==",
        t0.elapsed()
    );

    let acc = access(&storage);
    let cases = [
        ("1 hop, a.id pinned", "(a)-[r:k]->(b)", "a.id = 'n7'", false),
        ("1 hop, b.id pinned", "(a)-[r:k]->(b)", "b.id = 'n7'", false),
        (
            "{1,2} hops, a.id pinned",
            "(a)-[r:k]->{1,2}(b)",
            "a.id = 'n7'",
            true,
        ),
    ];
    for (name, pat, wh, var) in cases {
        let p = parse(pat, wh);
        for (route, hints) in [("global", SeekHints::none()), ("per-node", p.hints.clone())] {
            acc.reset();
            let t = Instant::now();
            let n = if var {
                // the unpinned route enumerates paths from EVERY node, which can
                // exceed the path cap; that is part of the answer
                let [PatternElement::Node(s), PatternElement::Relationship(r), PatternElement::Node(tg)] =
                    p.pattern.elements.as_slice()
                else {
                    unreachable!()
                };
                match execute_variable_length_with(
                    &acc,
                    s,
                    r,
                    tg,
                    &path_semantics_for(&p.pattern),
                    vec![VariableBinding::new()],
                    &hints,
                )
                .await
                {
                    Ok(v) => format!("{} rows", v.len()),
                    Err(e) => format!("error: {}", &e.to_string()[..60.min(e.to_string().len())]),
                }
            } else {
                format!("{} rows", hop(&acc, &p, &hints).await.len())
            };
            let (g, pn) = acc.counts();
            println!(
                "{name:28} {route:9} scanned {:>7} relations (global {g}, per-node {pn}) in {:>9.2?} -> {n}",
                g + pn,
                t.elapsed()
            );
        }
    }
    acc.reset();
    let t = Instant::now();
    let hits = neighborhood(
        &acc,
        &Open,
        &[(WS.to_string(), "n7".to_string())],
        &NeighborhoodParams {
            side: Side::Out,
            types: vec![],
            min_depth: 2,
            max_depth: 2,
            max_nodes: DEFAULT_MAX_NODES,
            max_visited: MAX_VISITED,
            budget: GraphBudget::standard(),
        },
    )
    .await
    .unwrap();
    println!(
        "{:28} {:9} scanned {:>7} relations in {:>9.2?} -> {} nodes",
        "NEIGHBORS depth 2",
        "set",
        acc.counts().1,
        t.elapsed(),
        hits.len()
    );
}

#[tokio::test]
#[ignore = "benchmark; prints numbers"]
async fn bench_pinned_reads() {
    bench_case(1_000, 10_000).await;
    bench_case(5_000, 100_000).await;
}

#[allow(dead_code)]
fn _keep(_: AuthContext) {}
