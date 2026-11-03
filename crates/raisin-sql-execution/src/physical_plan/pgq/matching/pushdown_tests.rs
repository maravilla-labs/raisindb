//! The pinned-endpoint pushdown: same answers as the global scan, a fraction of
//! the relations read.

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;

use raisin_sql::ast::pgq_parser::parse_graph_table;
use raisin_sql::ast::{NodePattern, PathPattern, PatternElement, RelationshipPattern};

use super::seek::SeekHints;
use super::single_hop::match_single_hop_with;
use super::variable_length::execute_variable_length_with;
use super::{path_semantics_for, PathSemantics};
use crate::physical_plan::graph_access::fake::FakeGraph;
use crate::physical_plan::pgq::types::VariableBinding;

const WS: &str = "w";

struct Parsed {
    source: NodePattern,
    rel: RelationshipPattern,
    target: NodePattern,
    semantics: PathSemantics,
    hints: SeekHints,
}

fn parse(match_sql: &str, where_sql: &str) -> Parsed {
    let filter = if where_sql.is_empty() {
        String::new()
    } else {
        format!(" WHERE {where_sql}")
    };
    let q = parse_graph_table(&format!(
        "GRAPH_TABLE(MATCH {match_sql}{filter} COLUMNS (a.id AS x))"
    ))
    .unwrap_or_else(|e| panic!("parse {match_sql}: {e:?}"));
    let pattern: &PathPattern = &q.match_clause.patterns[0];
    let [PatternElement::Node(source), PatternElement::Relationship(rel), PatternElement::Node(target)] =
        pattern.elements.as_slice()
    else {
        panic!("not a single hop: {match_sql}");
    };
    Parsed {
        source: source.clone(),
        rel: rel.clone(),
        target: target.clone(),
        semantics: path_semantics_for(pattern),
        hints: SeekHints::from_where(q.where_clause.as_ref().map(|w| &w.expression)),
    }
}

fn id_of(b: &VariableBinding, var: &str) -> String {
    b.get_node(var).map(|n| n.id.clone()).unwrap_or_default()
}

/// What a hop produced, as a comparable set: `(a, b, relation type)`.
fn hop_rows(bindings: &[VariableBinding]) -> BTreeSet<(String, String, String)> {
    bindings
        .iter()
        .map(|b| {
            (
                id_of(b, "a"),
                id_of(b, "b"),
                b.get_relation("r")
                    .map(|r| r.relation_type.clone())
                    .unwrap_or_default(),
            )
        })
        .collect()
}

async fn hop(g: &FakeGraph, p: &Parsed, hints: &SeekHints) -> Vec<VariableBinding> {
    match_single_hop_with(g, &p.source, &p.rel, &p.target, hints)
        .await
        .unwrap()
}

/// What the WHERE clause would keep: bindings whose pinned variable is the pin.
fn pinned(
    rows: BTreeSet<(String, String, String)>,
    a: Option<&str>,
    b: Option<&str>,
) -> BTreeSet<(String, String, String)> {
    rows.into_iter()
        .filter(|r| a.is_none_or(|x| r.0 == x) && b.is_none_or(|x| r.1 == x))
        .collect()
}

/// A tiny deterministic generator, so a failure names its seed.
struct Lcg(u64);
impl Lcg {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n.max(1)
    }
}

fn random_graph(seed: u64, n: usize, e: usize) -> FakeGraph {
    let mut rng = Lcg(seed);
    let mut g = FakeGraph::new();
    for i in 0..n {
        // Two labels, so label filters have something to filter.
        g.node(
            WS,
            &format!("n{i:03}"),
            if i % 3 == 0 { "t:Q" } else { "t:P" },
        );
    }
    let types = ["k", "f", "w"];
    for _ in 0..e {
        let a = format!("n{:03}", rng.below(n as u64));
        let b = format!("n{:03}", rng.below(n as u64));
        g.rel(WS, &a, &b, types[rng.below(3) as usize]);
    }
    g
}

#[tokio::test]
async fn a_pinned_source_reads_only_its_own_relations() {
    // 5,000 nodes in a ring, plus one node with 7 outgoing relations.
    let mut g = FakeGraph::new();
    for i in 0..5000 {
        g.node(WS, &format!("r{i}"), "t:P");
        g.rel(WS, &format!("r{i}"), &format!("r{}", (i + 1) % 5000), "k");
    }
    g.node(WS, "hub", "t:P");
    for i in 0..7 {
        g.rel(WS, "hub", &format!("r{i}"), "k");
    }

    let p = parse("(a)-[r:k]->(b)", "a.id = 'hub'");

    let global = hop(&g, &p, &SeekHints::none()).await;
    assert_eq!(g.global_read.load(Ordering::SeqCst), 5007);
    g.reset();

    let seeked = hop(&g, &p, &p.hints).await;
    assert_eq!(g.global_read.load(Ordering::SeqCst), 0, "no global scan");
    assert_eq!(g.global_scans.load(Ordering::SeqCst), 0);
    assert_eq!(
        g.node_read.load(Ordering::SeqCst),
        7,
        "hub's outgoing relations and nothing else (its incoming side is not needed)"
    );
    assert_eq!(
        hop_rows(&seeked),
        pinned(hop_rows(&global), Some("hub"), None)
    );
    assert_eq!(seeked.len(), 7);
}

#[tokio::test]
async fn a_pinned_target_uses_the_incoming_index() {
    let mut g = FakeGraph::new();
    for i in 0..1000 {
        g.node(WS, &format!("r{i}"), "t:P");
        g.rel(WS, &format!("r{i}"), &format!("r{}", (i + 1) % 1000), "k");
    }
    let p = parse("(a)-[r:k]->(b)", "b.id = 'r10'");
    let seeked = hop(&g, &p, &p.hints).await;
    assert_eq!(g.global_read.load(Ordering::SeqCst), 0);
    assert_eq!(g.node_read.load(Ordering::SeqCst), 1);
    assert_eq!(
        hop_rows(&seeked),
        BTreeSet::from([("r9".to_string(), "r10".to_string(), "k".to_string())])
    );
}

#[tokio::test]
async fn a_pin_on_either_end_is_enough_and_both_ends_read_once() {
    let mut g = FakeGraph::new();
    for i in 0..10 {
        g.node(WS, &format!("n{i}"), "t:P");
    }
    for i in 1..10 {
        g.rel(WS, "n0", &format!("n{i}"), "k");
    }
    let p = parse("(a)-[r:k]->(b)", "b.id = 'n5'");
    let seeked = hop(&g, &p, &p.hints).await;
    assert_eq!(g.node_calls.load(Ordering::SeqCst), 1);
    assert!(hop_rows(&seeked).contains(&("n0".into(), "n5".into(), "k".into())));
}

#[tokio::test]
async fn an_unknown_id_falls_back_to_the_global_scan() {
    let mut g = FakeGraph::new();
    g.node(WS, "a", "t:P")
        .node(WS, "b", "t:P")
        .rel(WS, "a", "b", "k");
    let p = parse("(a)-[r:k]->(b)", "a.id = 'ghost'");
    let rows = hop(&g, &p, &p.hints).await;
    assert_eq!(
        g.global_scans.load(Ordering::SeqCst),
        1,
        "exactness over speed"
    );
    // The filter, not the matcher, removes the non-matching relation.
    assert_eq!(pinned(hop_rows(&rows), Some("ghost"), None).len(), 0);
}

#[tokio::test]
async fn a_contradictory_pin_reads_nothing() {
    let mut g = FakeGraph::new();
    g.node(WS, "a", "t:P")
        .node(WS, "b", "t:P")
        .rel(WS, "a", "b", "k");
    let p = parse("(a)-[r:k]->(b)", "a.id = 'a' AND a.id = 'b'");
    assert!(hop(&g, &p, &p.hints).await.is_empty());
    assert_eq!(g.read_total(), 0);
    assert_eq!(g.global_scans.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn no_hint_means_the_global_scan_exactly_as_before() {
    let g = random_graph(7, 30, 80);
    let p = parse("(a)-[r:k]->(b)", "a.name = 'x' OR a.id = 'n001'");
    assert!(p.hints.seek_for(Some(&"a".to_string())).is_none());
    let _ = hop(&g, &p, &p.hints).await;
    assert_eq!(g.global_scans.load(Ordering::SeqCst), 1);
    assert_eq!(g.node_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn single_hop_matches_the_global_scan_on_random_graphs() {
    let patterns = [
        // direction x types x labels
        "(a)-[r:k]->(b)",
        "(a)<-[r:k]-(b)",
        "(a)-[r:k]-(b)",
        "(a)-[r]->(b)",
        "(a)<-[r]-(b)",
        "(a)-[r]-(b)",
        "(a)-[r:k|f]->(b)",
        "(a)<-[r:k|w]-(b)",
        "(a)-[r:f|w]-(b)",
        "(a:P)-[r:k]->(b:Q)",
        "(a:Q)<-[r:k|f]-(b:P)",
        "(a:P)-[r]-(b:P)",
    ];
    for (seed, n, e) in [(1, 6, 10), (2, 25, 70), (3, 80, 300), (4, 200, 600)] {
        let g = random_graph(seed, n, e);
        for pat in patterns {
            for pin_a in [true, false] {
                for pin_b in [true, false] {
                    if !pin_a && !pin_b {
                        continue;
                    }
                    for pick in 0..3u64 {
                        let id_a = format!("n{:03}", (pick * 7 + seed) % n as u64);
                        let id_b = format!("n{:03}", (pick * 11 + seed + 3) % n as u64);
                        let where_sql = match (pin_a, pin_b) {
                            (true, true) => format!("a.id = '{id_a}' AND b.id = '{id_b}'"),
                            (true, false) => format!("a.id = '{id_a}'"),
                            _ => format!("b.id = '{id_b}'"),
                        };
                        let p = parse(pat, &where_sql);
                        let global = hop(&g, &p, &SeekHints::none()).await;
                        let seeked = hop(&g, &p, &p.hints).await;
                        let want = pinned(
                            hop_rows(&global),
                            pin_a.then_some(id_a.as_str()),
                            pin_b.then_some(id_b.as_str()),
                        );
                        let got = pinned(
                            hop_rows(&seeked),
                            pin_a.then_some(id_a.as_str()),
                            pin_b.then_some(id_b.as_str()),
                        );
                        assert_eq!(got, want, "seed {seed} pattern {pat} where {where_sql}");
                    }
                }
            }
        }
    }
}

// ---- variable length -------------------------------------------------------

async fn var_rows(
    g: &FakeGraph,
    p: &Parsed,
    hints: &SeekHints,
) -> Result<BTreeSet<(String, String, usize)>, String> {
    let bindings = execute_variable_length_with(
        g,
        &p.source,
        &p.rel,
        &p.target,
        &p.semantics,
        vec![VariableBinding::new()],
        hints,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(bindings
        .iter()
        .map(|b| {
            (
                id_of(b, "a"),
                id_of(b, "b"),
                b.get_path("r").map(|x| x.length()).unwrap_or(usize::MAX),
            )
        })
        .collect())
}

#[tokio::test]
async fn a_pinned_variable_length_source_reads_only_what_it_reaches() {
    // Two disjoint 1,500-node rings; the pinned source sits in the first.
    let mut g = FakeGraph::new();
    for ring in ["x", "y"] {
        for i in 0..1500 {
            g.node(WS, &format!("{ring}{i}"), "t:P");
            g.rel(
                WS,
                &format!("{ring}{i}"),
                &format!("{ring}{}", (i + 1) % 1500),
                "k",
            );
        }
    }
    let p = parse("(a)-[r:k]->{1,3}(b)", "a.id = 'x0'");
    let seeked = var_rows(&g, &p, &p.hints).await.unwrap();
    assert_eq!(g.global_scans.load(Ordering::SeqCst), 0);
    // x0, x1, x2 expanded (depth 1..3 needs the edges of nodes within 2 hops).
    assert_eq!(g.node_read.load(Ordering::SeqCst), 3);
    assert_eq!(
        seeked,
        BTreeSet::from([
            ("x0".to_string(), "x1".to_string(), 1),
            ("x0".to_string(), "x2".to_string(), 2),
            ("x0".to_string(), "x3".to_string(), 3),
        ])
    );
    g.reset();
    let _ = var_rows(&g, &p, &SeekHints::none()).await.unwrap();
    assert_eq!(g.global_read.load(Ordering::SeqCst), 3000);
}

#[tokio::test]
async fn variable_length_matches_the_global_scan_on_random_graphs() {
    let patterns = [
        "(a)-[r:k]->{1,3}(b)",
        "(a)-[r:k]->{2,3}(b)",
        "(a)-[r:k]->{0,2}(b)",
        "(a)-[r:k]->{0,0}(b)",
        "(a)<-[r:k]-{1,3}(b)",
        "(a)-[r:k|f]->{1,3}(b)",
        "(a)-[r]->{1,2}(b)",
        "(a:P)-[r:k]->{1,3}(b:Q)",
        "(a)-[r:f]-{1,2}(b)",
        "ACYCLIC (a)-[r:k]->{1,4}(b)",
        "WALK (a)-[r:k]->{1,3}(b)",
        "TRAIL (a)-[r:k|f]->{1,3}(b)",
    ];
    for (seed, n, e) in [(11, 6, 9), (12, 20, 35), (13, 40, 70)] {
        let g = random_graph(seed, n, e);
        for pat in patterns {
            for pick in 0..4u64 {
                let id_a = format!("n{:03}", (pick * 5 + seed) % n as u64);
                let p = parse(pat, &format!("a.id = '{id_a}'"));
                let global = var_rows(&g, &p, &SeekHints::none()).await;
                let seeked = var_rows(&g, &p, &p.hints).await;
                let keep = |r: Result<BTreeSet<(String, String, usize)>, String>| {
                    r.map(|s| {
                        s.into_iter()
                            .filter(|x| x.0 == id_a)
                            .collect::<BTreeSet<_>>()
                    })
                };
                assert_eq!(
                    keep(seeked),
                    keep(global),
                    "seed {seed} pattern {pat} a={id_a}"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_pinned_source_with_no_edges_starts_nothing_exactly_like_the_global_route() {
    let mut g = FakeGraph::new();
    g.node(WS, "lonely", "t:P")
        .node(WS, "a", "t:P")
        .node(WS, "b", "t:P");
    g.rel(WS, "a", "b", "k");
    let p = parse("(a)-[r:k]->{0,2}(b)", "a.id = 'lonely'");
    let global = var_rows(&g, &p, &SeekHints::none()).await.unwrap();
    let seeked = var_rows(&g, &p, &p.hints).await.unwrap();
    assert!(global.iter().all(|r| r.0 != "lonely"));
    assert!(seeked.is_empty());
}

#[tokio::test]
async fn a_pinned_query_answers_where_the_unpinned_one_hits_the_path_cap() {
    // 6,000 starts x 3 paths = 18,000 paths: over the 10,000 cap when every node
    // is a start, but a pinned query only ever enumerates its own.
    let mut g = FakeGraph::new();
    for i in 0..6000 {
        g.node(WS, &format!("x{i}"), "t:P");
        g.rel(WS, &format!("x{i}"), &format!("x{}", (i + 1) % 6000), "k");
    }
    let p = parse("(a)-[r:k]->{1,3}(b)", "a.id = 'x0'");
    let err = var_rows(&g, &p, &SeekHints::none()).await.unwrap_err();
    assert!(err.contains("path cap"), "{err}");
    let ok = var_rows(&g, &p, &p.hints).await.unwrap();
    assert_eq!(ok.len(), 3);
}

/// Rows of a variable-length run as a sorted Vec, so multiplicity counts.
async fn var_list(
    g: &FakeGraph,
    p: &Parsed,
    hints: &SeekHints,
) -> Result<Vec<(String, String, usize)>, String> {
    var_rows_multi(g, p, hints).await
}

async fn var_rows_multi(
    g: &FakeGraph,
    p: &Parsed,
    hints: &SeekHints,
) -> Result<Vec<(String, String, usize)>, String> {
    let bindings = execute_variable_length_with(
        g,
        &p.source,
        &p.rel,
        &p.target,
        &p.semantics,
        vec![VariableBinding::new()],
        hints,
    )
    .await
    .map_err(|e| e.to_string())?;
    let mut rows: Vec<_> = bindings
        .iter()
        .map(|b| {
            (
                id_of(b, "a"),
                id_of(b, "b"),
                b.get_path("r").map(|x| x.length()).unwrap_or(usize::MAX),
            )
        })
        .collect();
    rows.sort();
    Ok(rows)
}

#[tokio::test]
async fn shortest_between_two_pinned_nodes_matches_the_global_route() {
    let patterns = [
        "ANY SHORTEST p = (a)-[r:k]->{1,4}(b)",
        "ALL SHORTEST p = (a)-[r:k|f]->{1,3}(b)",
        "ANY SHORTEST p = (a)<-[r:k]-{1,3}(b)",
        "ALL SHORTEST p = (a:P)-[r]->{1,3}(b:Q)",
    ];
    for (seed, n, e) in [(31, 8, 14), (32, 25, 60), (33, 40, 120)] {
        let g = random_graph(seed, n, e);
        for pat in patterns {
            for pick in 0..5u64 {
                let ida = format!("n{:03}", (pick * 3 + seed) % n as u64);
                let idb = format!("n{:03}", (pick * 7 + seed + 1) % n as u64);
                let p = parse(pat, &format!("a.id = '{ida}' AND b.id = '{idb}'"));
                let keep = |r: Result<Vec<(String, String, usize)>, String>| {
                    r.map(|v| {
                        v.into_iter()
                            .filter(|x| x.0 == ida && x.1 == idb && ida != idb)
                            .collect::<Vec<_>>()
                    })
                };
                let global = var_list(&g, &p, &SeekHints::none()).await;
                let seeked = var_list(&g, &p, &p.hints).await;
                assert_eq!(keep(seeked), keep(global), "seed {seed} {pat} {ida}->{idb}");
            }
        }
    }
}

#[tokio::test]
async fn shortest_between_two_pinned_nodes_does_not_enumerate_paths() {
    // A complete graph on 30 nodes: roughly 30^4 paths of length <= 4 from any
    // node, far past the 10,000 path cap. The shortest path is one hop.
    let mut g = FakeGraph::new();
    for i in 0..30 {
        g.node(WS, &format!("c{i}"), "t:P");
    }
    for i in 0..30 {
        for j in 0..30 {
            if i != j {
                g.rel(WS, &format!("c{i}"), &format!("c{j}"), "k");
            }
        }
    }
    let p = parse(
        "ANY SHORTEST p = (a)-[r:k]->{1,4}(b)",
        "a.id = 'c0' AND b.id = 'c17'",
    );
    let err = var_list(&g, &p, &SeekHints::none()).await.unwrap_err();
    assert!(err.contains("path cap"), "{err}");
    let rows = var_list(&g, &p, &p.hints).await.unwrap();
    assert_eq!(rows, vec![("c0".into(), "c17".into(), 1)]);
}
