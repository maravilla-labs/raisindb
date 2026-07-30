//! Tests for path variables, selectors and restrictors.

use super::parse_graph_table;
use crate::ast::pgq::*;

fn parse_ok(sql: &str) -> GraphTableQuery {
    parse_graph_table(sql).unwrap_or_else(|e| panic!("expected {sql} to parse, got: {e}"))
}

fn reject(sql: &str) -> String {
    match parse_graph_table(sql) {
        Ok(_) => panic!("expected {sql} to be rejected"),
        Err(e) => e.message,
    }
}

fn first_path(q: &GraphTableQuery) -> &PathPattern {
    &q.match_clause.patterns[0]
}

// ---------------------------------------------------------------- path variable

#[test]
fn path_variable_before_the_pattern() {
    let q = parse_ok("GRAPH_TABLE(MATCH p = (a)-[:knows]->(b) COLUMNS (path_length(p)))");
    assert_eq!(first_path(&q).variable, Some("p".into()));
    assert!(first_path(&q).selector.is_none());
}

#[test]
fn path_variable_is_optional() {
    let q = parse_ok("GRAPH_TABLE(MATCH (a)-[:knows]->(b) COLUMNS (a.id))");
    assert_eq!(first_path(&q).variable, None);
}

#[test]
fn path_variable_after_selector_duckpgq_style() {
    let q = parse_ok("GRAPH_TABLE(MATCH p = ANY SHORTEST (a)-[:knows]->+(b) COLUMNS (nodes(p)))");
    assert_eq!(first_path(&q).variable, Some("p".into()));
    assert_eq!(first_path(&q).selector, Some(PathSelector::AnyShortest));
}

#[test]
fn path_variable_after_restrictor_committee_style() {
    let q = parse_ok(
        "GRAPH_TABLE(MATCH ALL SHORTEST TRAIL p = (a)-[t:knows]->{1,3}(b) COLUMNS (edges(p)))",
    );
    let path = first_path(&q);
    assert_eq!(path.variable, Some("p".into()));
    assert_eq!(path.selector, Some(PathSelector::AllShortest));
    assert_eq!(path.restrictor, Some(PathRestrictor::Trail));
}

#[test]
fn path_variable_twice_is_rejected() {
    let msg = reject("GRAPH_TABLE(MATCH p = ANY SHORTEST q = (a)-[:k]->(b) COLUMNS (a.id))");
    assert!(msg.contains("path variable given twice"), "{msg}");
}

#[test]
fn each_comma_separated_path_carries_its_own_variable() {
    let q = parse_ok(
        "GRAPH_TABLE(MATCH p = (a)-[:k]->(b), q = ANY SHORTEST (c)-[:k]->{1,2}(d) \
         COLUMNS (path_length(p), path_length(q)))",
    );
    assert_eq!(q.match_clause.patterns.len(), 2);
    assert_eq!(q.match_clause.patterns[0].variable, Some("p".into()));
    assert_eq!(q.match_clause.patterns[1].variable, Some("q".into()));
    assert_eq!(
        q.match_clause.patterns[1].selector,
        Some(PathSelector::AnyShortest)
    );
}

// ------------------------------------------------------------------- selectors

#[test]
fn every_supported_selector_parses() {
    let cases = [
        ("ANY SHORTEST", PathSelector::AnyShortest),
        ("ALL SHORTEST", PathSelector::AllShortest),
        ("ANY", PathSelector::Any),
    ];
    for (spelling, expected) in cases {
        let sql = format!("GRAPH_TABLE(MATCH {spelling} p = (a)-[:k]->{{1,3}}(b) COLUMNS (a.id))");
        assert_eq!(first_path(&parse_ok(&sql)).selector, Some(expected));
    }
}

#[test]
fn selectors_are_case_insensitive() {
    let q = parse_ok("GRAPH_TABLE(MATCH any shortest p = (a)-[:k]->{1,3}(b) COLUMNS (a.id))");
    assert_eq!(first_path(&q).selector, Some(PathSelector::AnyShortest));
}

/// A bare-`ANY`-first alternative would truncate `ANY SHORTEST` to `ANY` and
/// then die on the leftover keyword.
#[test]
fn any_shortest_is_not_truncated_to_any() {
    let q = parse_ok("GRAPH_TABLE(MATCH ANY SHORTEST p = (a)-[:k]->{1,3}(b) COLUMNS (a.id))");
    assert_eq!(first_path(&q).selector, Some(PathSelector::AnyShortest));
    assert_eq!(first_path(&q).elements.len(), 3);
}

#[test]
fn a_variable_starting_with_a_keyword_is_not_a_selector() {
    let q = parse_ok("GRAPH_TABLE(MATCH (anyone)-[:k]->(b) COLUMNS (anyone.id))");
    match &first_path(&q).elements[0] {
        PatternElement::Node(n) => assert_eq!(n.variable, Some("anyone".into())),
        other => panic!("expected node, got {other:?}"),
    }
    assert!(first_path(&q).selector.is_none());
}

#[test]
fn deferred_selectors_are_named_not_ignored() {
    for (sql, needle) in [
        (
            "GRAPH_TABLE(MATCH SHORTEST 3 p = (a)-[:k]->{1,3}(b) COLUMNS (a.id))",
            "SHORTEST k",
        ),
        (
            "GRAPH_TABLE(MATCH SHORTEST 3 GROUP p = (a)-[:k]->{1,3}(b) COLUMNS (a.id))",
            "SHORTEST k GROUP",
        ),
        (
            "GRAPH_TABLE(MATCH ANY 2 p = (a)-[:k]->{1,3}(b) COLUMNS (a.id))",
            "ANY k",
        ),
    ] {
        let msg = reject(sql);
        assert!(msg.contains(needle), "{msg}");
        assert!(msg.contains("not supported yet"), "{msg}");
    }
}

// ----------------------------------------------------------------- restrictors

#[test]
fn every_supported_restrictor_parses() {
    let cases = [
        ("WALK", PathRestrictor::Walk),
        ("TRAIL", PathRestrictor::Trail),
        ("ACYCLIC", PathRestrictor::Acyclic),
    ];
    for (spelling, expected) in cases {
        let sql = format!("GRAPH_TABLE(MATCH {spelling} p = (a)-[:k]->*(b) COLUMNS (a.id))");
        assert_eq!(first_path(&parse_ok(&sql)).restrictor, Some(expected));
    }
}

#[test]
fn restrictor_defaults_to_acyclic_when_unwritten() {
    let q = parse_ok("GRAPH_TABLE(MATCH (a)-[:k]->{1,3}(b) COLUMNS (a.id))");
    assert_eq!(first_path(&q).restrictor, None);
    assert_eq!(
        first_path(&q).effective_restrictor(),
        PathRestrictor::Acyclic
    );
}

#[test]
fn simple_restrictor_is_named_not_aliased_to_acyclic() {
    let msg = reject("GRAPH_TABLE(MATCH SIMPLE p = (a)-[:k]->*(b) COLUMNS (a.id))");
    assert!(msg.contains("SIMPLE"), "{msg}");
    assert!(msg.contains("ACYCLIC"), "{msg}");
    assert!(msg.contains("TRAIL"), "{msg}");
}

#[test]
fn restrictor_before_selector_names_the_correct_order() {
    let msg = reject("GRAPH_TABLE(MATCH TRAIL ANY SHORTEST p = (a)-[:k]->*(b) COLUMNS (a.id))");
    assert!(msg.contains("must come after the path selector"), "{msg}");
    assert!(msg.contains("ANY SHORTEST TRAIL"), "{msg}");
}
