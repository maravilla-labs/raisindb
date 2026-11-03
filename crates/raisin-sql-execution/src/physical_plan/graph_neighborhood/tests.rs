use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::atomic::Ordering;

use std::time::{Duration, Instant};

use super::*;
use crate::physical_plan::graph_access::fake::FakeGraph;

const WS: &str = "w";

struct Open;
impl NodeGate for Open {
    type Item = ();
    async fn report(&self, _: &str, _: &str, _: u32) -> Result<Option<()>> {
        Ok(Some(()))
    }
}

/// Hides a fixed set of nodes, the way row-level security would.
struct Hide(HashSet<String>);
impl NodeGate for Hide {
    type Item = ();
    async fn report(&self, _: &str, id: &str, _: u32) -> Result<Option<()>> {
        Ok((!self.0.contains(id)).then_some(()))
    }
}

fn params(side: Side, max_depth: u32) -> NeighborhoodParams {
    NeighborhoodParams {
        side,
        types: vec![],
        min_depth: 1,
        max_depth,
        max_nodes: DEFAULT_MAX_NODES,
        max_visited: MAX_VISITED,
        budget: GraphBudget::standard(),
    }
}

fn start(id: &str) -> Vec<NodeKey> {
    vec![(WS.to_string(), id.to_string())]
}

fn graph(edges: &[(&str, &str, &str)]) -> FakeGraph {
    let mut g = FakeGraph::new();
    for (a, b, _) in edges {
        g.node(WS, a, "t:P");
        g.node(WS, b, "t:P");
    }
    for (a, b, ty) in edges {
        g.rel(WS, a, b, ty);
    }
    g
}

fn rows(hits: &[Hit<()>]) -> Vec<(String, u32, u64)> {
    hits.iter()
        .map(|h| (h.id.clone(), h.depth, h.via_count))
        .collect()
}

async fn run(g: &FakeGraph, from: &str, p: &NeighborhoodParams) -> Vec<(String, u32, u64)> {
    rows(&neighborhood(g, &Open, &start(from), p).await.unwrap())
}

#[tokio::test]
async fn a_line_reports_each_node_at_its_distance() {
    let g = graph(&[("a", "b", "k"), ("b", "c", "k"), ("c", "d", "k")]);
    assert_eq!(
        run(&g, "a", &params(Side::Out, 3)).await,
        vec![("b".into(), 1, 1), ("c".into(), 2, 1), ("d".into(), 3, 1)]
    );
    assert_eq!(
        run(&g, "a", &params(Side::Out, 2)).await,
        vec![("b".into(), 1, 1), ("c".into(), 2, 1)]
    );
}

#[tokio::test]
async fn via_count_is_the_number_of_mutual_connections() {
    // me knows x, y, z; x and y know t; z knows u.
    let g = graph(&[
        ("me", "x", "k"),
        ("me", "y", "k"),
        ("me", "z", "k"),
        ("x", "t", "k"),
        ("y", "t", "k"),
        ("z", "u", "k"),
    ]);
    let mut p = params(Side::Out, 2);
    p.min_depth = 2;
    assert_eq!(
        run(&g, "me", &p).await,
        vec![("t".into(), 2, 2), ("u".into(), 2, 1)]
    );
}

#[tokio::test]
async fn a_node_is_reported_once_at_its_shortest_distance() {
    // d is reachable at depth 2 (a-b-d) and at depth 3 (a-c-e-d).
    let g = graph(&[
        ("a", "b", "k"),
        ("a", "c", "k"),
        ("b", "d", "k"),
        ("c", "e", "k"),
        ("e", "d", "k"),
    ]);
    let rows = run(&g, "a", &params(Side::Out, 4)).await;
    assert_eq!(rows.iter().filter(|r| r.0 == "d").count(), 1);
    assert_eq!(
        rows.iter().find(|r| r.0 == "d").map(|r| (r.1, r.2)),
        Some((2, 1)),
        "only b is one level closer and reaches d"
    );
}

#[tokio::test]
async fn the_start_cycles_and_self_loops_are_never_reported() {
    let g = graph(&[("a", "b", "k"), ("b", "a", "k"), ("a", "a", "k")]);
    assert_eq!(
        run(&g, "a", &params(Side::Out, 5)).await,
        vec![("b".into(), 1, 1)]
    );
}

#[tokio::test]
async fn parallel_relations_between_the_same_pair_count_once() {
    let g = graph(&[
        ("a", "b", "k"),
        ("a", "b", "f"),
        ("b", "c", "k"),
        ("b", "c", "f"),
    ]);
    assert_eq!(
        run(&g, "a", &params(Side::Out, 2)).await,
        vec![("b".into(), 1, 1), ("c".into(), 2, 1)]
    );
}

#[tokio::test]
async fn both_directions_count_a_mutual_pair_once() {
    // a<->b stored in both directions: with Both the pair is one mutual link.
    let g = graph(&[
        ("a", "b", "k"),
        ("b", "a", "k"),
        ("b", "c", "k"),
        ("c", "b", "k"),
    ]);
    assert_eq!(
        run(&g, "a", &params(Side::Both, 2)).await,
        vec![("b".into(), 1, 1), ("c".into(), 2, 1)]
    );
}

#[tokio::test]
async fn in_follows_relations_backwards() {
    let g = graph(&[("a", "b", "k"), ("b", "c", "k")]);
    assert_eq!(
        run(&g, "c", &params(Side::In, 2)).await,
        vec![("b".into(), 1, 1), ("a".into(), 2, 1)]
    );
    assert!(run(&g, "c", &params(Side::Out, 2)).await.is_empty());
}

#[tokio::test]
async fn types_restrict_what_is_followed() {
    let g = graph(&[
        ("a", "b", "k"),
        ("a", "c", "f"),
        ("b", "d", "f"),
        ("c", "e", "f"),
    ]);
    let mut p = params(Side::Out, 3);
    p.types = vec!["f".into()];
    assert_eq!(
        run(&g, "a", &p).await,
        vec![("c".into(), 1, 1), ("e".into(), 2, 1)]
    );
    p.types = vec!["k".into(), "f".into()];
    assert_eq!(run(&g, "a", &p).await.len(), 4);
}

#[tokio::test]
async fn min_depth_hides_near_nodes_but_still_walks_through_them() {
    let g = graph(&[("a", "b", "k"), ("b", "c", "k"), ("c", "d", "k")]);
    let mut p = params(Side::Out, 3);
    p.min_depth = 3;
    assert_eq!(run(&g, "a", &p).await, vec![("d".into(), 3, 1)]);
}

#[tokio::test]
async fn an_unknown_start_or_one_without_relations_is_empty() {
    let g = graph(&[("a", "b", "k")]);
    assert!(run(&g, "zzz", &params(Side::Out, 3)).await.is_empty());
    assert!(run(&g, "b", &params(Side::Out, 3)).await.is_empty());
}

#[tokio::test]
async fn each_visited_node_costs_one_index_seek_never_a_scan() {
    // A hub: a -> 50 spokes, every spoke -> every other spoke. Path enumeration
    // would explode; the set search reads each node once.
    let mut edges: Vec<(String, String)> = Vec::new();
    for i in 0..50 {
        edges.push(("a".into(), format!("s{i}")));
        for j in 0..50 {
            if i != j {
                edges.push((format!("s{i}"), format!("s{j}")));
            }
        }
    }
    let mut g = FakeGraph::new();
    g.node(WS, "a", "t:P");
    for i in 0..50 {
        g.node(WS, &format!("s{i}"), "t:P");
    }
    for (x, y) in &edges {
        g.rel(WS, x, y, "k");
    }
    let rows = run(&g, "a", &params(Side::Out, 6)).await;
    assert_eq!(rows.len(), 50);
    assert!(rows.iter().all(|r| r.1 == 1 && r.2 == 1));
    // a + 50 spokes expanded once each; no global scan at all.
    assert_eq!(g.node_calls.load(Ordering::SeqCst), 51);
    assert_eq!(g.global_scans.load(Ordering::SeqCst), 0);
}

/// No digit of any count, and nothing but the fixed text.
fn assert_generic(err: &ExecutionError, text: &str) {
    let shown = err.to_string();
    assert!(shown.contains(text), "{shown}");
    assert!(
        !shown.chars().any(|c| c.is_ascii_digit()),
        "the error must not carry counts: {shown}"
    );
}

#[tokio::test]
async fn the_visited_budget_fails_it_does_not_truncate() {
    let g = graph(&[("a", "b", "k"), ("a", "c", "k"), ("a", "d", "k")]);
    let mut p = params(Side::Out, 2);

    // Start + 3 neighbours = 4 visited: exactly the budget is fine.
    p.max_nodes = 4;
    assert_eq!(run(&g, "a", &p).await.len(), 3);

    p.max_nodes = 3;
    let err = neighborhood(&g, &Open, &start("a"), &p).await.unwrap_err();
    assert_generic(&err, TOO_LARGE);
}

#[tokio::test]
async fn the_relation_ceiling_counts_every_relation_read() {
    // a has 3 followed and 3 unfollowed relations; b, c, d have none.
    let g = graph(&[
        ("a", "b", "k"),
        ("a", "c", "k"),
        ("a", "d", "k"),
        ("a", "x", "f"),
        ("a", "y", "f"),
        ("a", "z", "f"),
    ]);
    let mut p = params(Side::Out, 2);
    p.types = vec!["k".to_string()];

    p.budget = GraphBudget::new(Duration::from_secs(60), 6);
    assert_eq!(
        run(&g, "a", &p).await.len(),
        3,
        "six relations read: at the ceiling"
    );

    p.budget = GraphBudget::new(Duration::from_secs(60), 5);
    let err = neighborhood(&g, &Open, &start("a"), &p).await.unwrap_err();
    assert_generic(&err, TOO_LARGE);
}

#[tokio::test]
async fn the_deadline_fails_the_search() {
    let g = graph(&[("a", "b", "k"), ("b", "c", "k")]);
    let mut p = params(Side::Out, 2);
    p.budget = GraphBudget::until(Instant::now(), 1_000);
    let err = neighborhood(&g, &Open, &start("a"), &p).await.unwrap_err();
    assert_generic(&err, TOO_SLOW);
    // Nothing was read from the index once the clock had run out.
    assert_eq!(g.node_calls.load(Ordering::SeqCst), 0);
}

/// a -> r1..r3 (readable); every r has `hidden` private nodes behind it, and
/// each private node leads to one readable leaf.
fn readable_and_hidden(hidden: usize) -> (FakeGraph, Hide) {
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut private = HashSet::new();
    for r in 1..=3 {
        edges.push(("a".into(), format!("r{r}")));
        for h in 0..hidden {
            let hid = format!("h{r}_{h}");
            edges.push((format!("r{r}"), hid.clone()));
            edges.push((hid.clone(), format!("leaf{r}")));
            private.insert(hid);
        }
    }
    let mut g = FakeGraph::new();
    for (x, y) in &edges {
        g.node(WS, x, "t:P").node(WS, y, "t:P");
    }
    for (x, y) in &edges {
        g.rel(WS, x, y, "k");
    }
    (g, Hide(private))
}

#[tokio::test]
async fn the_budget_trip_point_does_not_depend_on_hidden_nodes() {
    // Readable: a, r1..r3, leaf1..leaf3 = 7, whatever hides behind them.
    for hidden in [1, 5, 40] {
        let (g, gate) = readable_and_hidden(hidden);
        let mut p = params(Side::Out, 3);
        p.max_nodes = 7;
        let hits = neighborhood(&g, &gate, &start("a"), &p).await.unwrap();
        assert_eq!(hits.len(), 6, "hidden = {hidden}");
        p.max_nodes = 6;
        let err = neighborhood(&g, &gate, &start("a"), &p).await.unwrap_err();
        assert_generic(&err, TOO_LARGE);
    }
}

#[tokio::test]
async fn hidden_nodes_are_charged_to_the_fixed_ceiling_with_the_same_error() {
    let (g, gate) = readable_and_hidden(40);
    let mut p = params(Side::Out, 3);
    // 7 readable + 120 hidden = 127 visited.
    p.max_visited = 127;
    assert_eq!(
        neighborhood(&g, &gate, &start("a"), &p)
            .await
            .unwrap()
            .len(),
        6
    );
    p.max_visited = 126;
    let err = neighborhood(&g, &gate, &start("a"), &p).await.unwrap_err();
    assert_generic(&err, TOO_LARGE);
    let mut q = params(Side::Out, 3);
    q.max_nodes = 6;
    let caller = neighborhood(&g, &gate, &start("a"), &q).await.unwrap_err();
    assert_eq!(err.to_string(), caller.to_string(), "one text for both");
}

#[tokio::test]
async fn an_unreadable_start_is_the_same_as_a_missing_one() {
    let g = graph(&[("a", "b", "k"), ("b", "c", "k")]);
    // The real gate answers `None` for a node that does not exist, too.
    let gate = Hide(["a".to_string(), "ghost".to_string()].into());
    let hidden = neighborhood(&g, &gate, &start("a"), &params(Side::Out, 3))
        .await
        .unwrap();
    let missing = neighborhood(&g, &gate, &start("ghost"), &params(Side::Out, 3))
        .await
        .unwrap();
    assert!(hidden.is_empty());
    assert!(missing.is_empty());
    assert_eq!(
        g.node_calls.load(Ordering::SeqCst),
        0,
        "nothing is read from a start the caller cannot read"
    );
    // Even a budget the walk would blow is not reached: nothing is walked.
    let mut tight = params(Side::Out, 3);
    tight.max_nodes = 1;
    assert!(neighborhood(&g, &gate, &start("a"), &tight)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_hidden_node_is_not_reported_but_is_walked_through() {
    // a-h-t and a-v-t: t is reachable through h (hidden) and v (visible).
    let g = graph(&[
        ("a", "h", "k"),
        ("a", "v", "k"),
        ("h", "t", "k"),
        ("v", "t", "k"),
        ("h", "only", "k"),
    ]);
    let gate = Hide(["h".to_string()].into());
    let hits = neighborhood(&g, &gate, &start("a"), &params(Side::Out, 3))
        .await
        .unwrap();
    assert_eq!(
        rows(&hits),
        vec![
            ("v".into(), 1, 1),
            ("only".into(), 2, 0),
            ("t".into(), 2, 1)
        ],
        "h is left out, `only` (reachable only through h) is still found, and \
         via_count counts readable intermediates only"
    );
}

#[tokio::test]
async fn via_count_excludes_hidden_nodes_even_below_min_depth() {
    // The level before min_depth is never reported, but its readability still
    // decides via_count.
    let g = graph(&[
        ("a", "h1", "k"),
        ("a", "h2", "k"),
        ("a", "v", "k"),
        ("h1", "t", "k"),
        ("h2", "t", "k"),
        ("v", "t", "k"),
    ]);
    let gate = Hide(["h1".to_string(), "h2".to_string()].into());
    let mut p = params(Side::Out, 2);
    p.min_depth = 2;
    let hits = neighborhood(&g, &gate, &start("a"), &p).await.unwrap();
    assert_eq!(rows(&hits), vec![("t".into(), 2, 1)]);
    let open = neighborhood(&g, &Open, &start("a"), &p).await.unwrap();
    assert_eq!(rows(&open), vec![("t".into(), 2, 3)]);
}

/// Counts the nodes the gate is asked about.
struct Counting(std::sync::atomic::AtomicUsize);
impl NodeGate for Counting {
    type Item = ();
    async fn report(&self, _: &str, _: &str, _: u32) -> Result<Option<()>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Some(()))
    }
}

#[tokio::test]
async fn results_come_out_in_chunks_and_every_node_is_checked_once_when_reached() {
    // a -> 600 spokes -> one tail each.
    let mut g = FakeGraph::new();
    g.node(WS, "a", "t:P");
    for i in 0..600 {
        let s = format!("s{i:03}");
        let t = format!("t{i:03}");
        g.node(WS, &s, "t:P").node(WS, &t, "t:P");
        g.rel(WS, "a", &s, "k").rel(WS, &s, &t, "k");
    }
    let mut p = params(Side::Out, 3);
    p.min_depth = 3;
    let gate = Counting(Default::default());
    let mut search = NeighborhoodSearch::start(&g, &gate, &start("a"), &p)
        .await
        .unwrap();
    assert!(
        search.next_chunk().await.unwrap().is_none(),
        "nothing at depth 3"
    );
    // The start, then every spoke and every tail once when reached.
    assert_eq!(gate.0.load(Ordering::SeqCst), 1 + 600 + 600);

    p.min_depth = 2;
    let gate = Open;
    let mut search = NeighborhoodSearch::start(&g, &gate, &start("a"), &p)
        .await
        .unwrap();
    let mut sizes = Vec::new();
    while let Some(chunk) = search.next_chunk().await.unwrap() {
        sizes.push(chunk.len());
    }
    assert_eq!(sizes, vec![CHUNK, CHUNK, 600 - 2 * CHUNK]);
}

// ---- oracle ---------------------------------------------------------------

/// A tiny deterministic generator, so a failure names its seed.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Reference implementation written for obviousness, not speed: explicit
/// distances by repeated relaxation over the whole edge list.
#[allow(clippy::too_many_arguments)]
fn oracle(
    nodes: &[String],
    edges: &[(String, String, String)],
    start: &str,
    side: Side,
    types: &[String],
    min_depth: u32,
    max_depth: u32,
    hidden: &HashSet<String>,
) -> BTreeMap<String, (u32, u64)> {
    let follows = |t: &str| types.is_empty() || types.iter().any(|w| w == t);
    let mut adj: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (a, b, t) in edges {
        if !follows(t) {
            continue;
        }
        if matches!(side, Side::Out | Side::Both) {
            adj.entry(a).or_default().insert(b);
        }
        if matches!(side, Side::In | Side::Both) {
            adj.entry(b).or_default().insert(a);
        }
    }
    let mut dist: BTreeMap<&str, u32> = BTreeMap::new();
    dist.insert(start, 0);
    for d in 0..max_depth {
        let layer: Vec<&str> = dist
            .iter()
            .filter(|(_, &x)| x == d)
            .map(|(k, _)| *k)
            .collect();
        for u in layer {
            for v in adj.get(u).into_iter().flatten() {
                if !dist.contains_key(v) {
                    dist.insert(v, d + 1);
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    if hidden.contains(start) {
        return out;
    }
    for n in nodes {
        if let Some(&d) = dist.get(n.as_str()) {
            if d >= 1 && d >= min_depth && !hidden.contains(n) {
                let via = dist
                    .iter()
                    .filter(|(_, &x)| x == d - 1)
                    .filter(|(u, _)| !hidden.contains(**u))
                    .filter(|(u, _)| adj.get(*u).is_some_and(|s| s.contains(n.as_str())))
                    .count() as u64;
                out.insert(n.clone(), (d, via));
            }
        }
    }
    out
}

async fn check_random(seed: u64, n: usize, edge_count: usize, undirected: bool, multi: bool) {
    let mut rng = Lcg(seed);
    let nodes: Vec<String> = (0..n).map(|i| format!("n{i:03}")).collect();
    let type_names = ["k", "f", "w"];
    let mut edges: Vec<(String, String, String)> = Vec::new();
    for _ in 0..edge_count {
        let a = &nodes[rng.below(n as u64) as usize];
        let b = &nodes[rng.below(n as u64) as usize];
        let t = if multi {
            type_names[rng.below(3) as usize]
        } else {
            "k"
        };
        edges.push((a.clone(), b.clone(), t.to_string()));
        if undirected {
            edges.push((b.clone(), a.clone(), t.to_string()));
        }
    }
    let mut g = FakeGraph::new();
    for id in &nodes {
        g.node(WS, id, "t:P");
    }
    for (a, b, t) in &edges {
        g.rel(WS, a, b, t);
    }

    let hidden: HashSet<String> = nodes
        .iter()
        .filter(|_| rng.below(10) == 0)
        .cloned()
        .collect();

    for side in [Side::Out, Side::In, Side::Both] {
        for types in [
            vec![],
            vec!["k".to_string()],
            vec!["f".to_string(), "w".to_string()],
        ] {
            for (min_depth, max_depth) in [(1, 1), (1, 2), (2, 2), (1, 4), (2, 4)] {
                for who in 0..3usize {
                    let from = &nodes[rng.below(n as u64) as usize];
                    let p = NeighborhoodParams {
                        side,
                        types: types.clone(),
                        min_depth,
                        max_depth,
                        max_nodes: DEFAULT_MAX_NODES,
                        max_visited: MAX_VISITED,
                        budget: GraphBudget::standard(),
                    };
                    let (hits, hide) = if who == 0 {
                        (
                            neighborhood(&g, &Hide(hidden.clone()), &start(from), &p)
                                .await
                                .unwrap(),
                            hidden.clone(),
                        )
                    } else {
                        (
                            neighborhood(&g, &Open, &start(from), &p).await.unwrap(),
                            HashSet::new(),
                        )
                    };
                    let got: BTreeMap<String, (u32, u64)> = hits
                        .iter()
                        .map(|h| (h.id.clone(), (h.depth, h.via_count)))
                        .collect();
                    assert_eq!(
                        hits.len(),
                        got.len(),
                        "seed {seed}: a node was reported twice"
                    );
                    let want = oracle(
                        &nodes, &edges, from, side, &types, min_depth, max_depth, &hide,
                    );
                    assert_eq!(
                        got,
                        want,
                        "seed {seed} n={n} e={edge_count} undirected={undirected} multi={multi} \
                         side={side:?} types={types:?} depth={min_depth}..{max_depth} from={from} \
                         hidden={}",
                        !hide.is_empty()
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn matches_the_oracle_on_random_graphs() {
    // Several sizes and densities, directed and undirected, single and
    // multi-type, with and without hidden nodes.
    for (seed, n, e) in [
        (1, 8, 12),
        (2, 20, 40),
        (3, 60, 150),
        (4, 120, 200),
        (5, 200, 700),
    ] {
        for undirected in [false, true] {
            for multi in [false, true] {
                check_random(seed * 1000 + n as u64, n, e, undirected, multi).await;
            }
        }
    }
}
