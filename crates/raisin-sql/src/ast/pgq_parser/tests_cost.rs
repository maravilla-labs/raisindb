//! Tests for `COST` / `ANY CHEAPEST`, the inline-WHERE rejection, relationship
//! type alternation, and diagnostic quality.

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

fn relationship(q: &GraphTableQuery, index: usize) -> &RelationshipPattern {
    match &first_path(q).elements[index] {
        PatternElement::Relationship(r) => r,
        other => panic!("expected relationship at {index}, got {other:?}"),
    }
}

// ------------------------------------------------------- ANY CHEAPEST / COST

#[test]
fn any_cheapest_with_cost_parses() {
    let q = parse_ok(
        "GRAPH_TABLE(MATCH ANY CHEAPEST p = (a:Account)-[t:Transfers COST t.weight]->{1,3}\
         (b:Account) COLUMNS (path_length(p)))",
    );
    assert_eq!(first_path(&q).selector, Some(PathSelector::AnyCheapest));
    let rel = relationship(&q, 1);
    assert_eq!(rel.variable, Some("t".into()));
    match rel.cost.as_deref() {
        Some(Expr::PropertyAccess {
            variable,
            properties,
            ..
        }) => {
            assert_eq!(variable, "t");
            assert_eq!(properties, &vec!["weight".to_string()]);
        }
        other => panic!("expected t.weight, got {other:?}"),
    }
}

#[test]
fn cost_accepts_a_positive_numeric_literal() {
    let q = parse_ok(
        "GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[t:T COST 1]->{1,3}(b) COLUMNS (path_length(p)))",
    );
    assert!(relationship(&q, 1).cost.is_some());
}

#[test]
fn cost_before_the_legacy_quantifier_also_parses() {
    let q = parse_ok(
        "GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[t:T COST t.weight*1..3]->(b) \
         COLUMNS (path_length(p)))",
    );
    assert!(relationship(&q, 1).cost.is_some());
    assert!(relationship(&q, 1).quantifier.unwrap().is_legacy());
}

#[test]
fn cost_on_a_non_weight_property_is_rejected_with_the_relation_shape() {
    let msg = reject(
        "GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[t:T COST t.amount]->{1,3}(b) COLUMNS (a.id))",
    );
    assert!(msg.contains("no property map"), "{msg}");
    assert!(msg.contains("COST t.weight"), "{msg}");
}

#[test]
fn cost_on_an_anonymous_edge_is_rejected() {
    let msg = reject(
        "GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[:T COST t.weight]->{1,3}(b) COLUMNS (a.id))",
    );
    assert!(msg.contains("bound edge variable"), "{msg}");
}

#[test]
fn cost_referencing_a_different_variable_is_rejected() {
    let msg = reject(
        "GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[t:T COST u.weight]->{1,3}(b) COLUMNS (a.id))",
    );
    assert!(msg.contains("bound to 't'"), "{msg}");
}

#[test]
fn cost_with_a_non_positive_literal_is_rejected() {
    let msg =
        reject("GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[t:T COST 0]->{1,3}(b) COLUMNS (a.id))");
    assert!(msg.contains("positive finite number"), "{msg}");
}

/// Silently answering a weighted question by hop count is exactly the
/// wrong-results class this grammar exists to avoid.
#[test]
fn any_cheapest_without_cost_is_rejected() {
    let msg = reject("GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[t:T]->{1,3}(b) COLUMNS (a.id))");
    assert!(msg.contains("requires a COST clause"), "{msg}");
    assert!(msg.contains("ANY SHORTEST"), "{msg}");
}

#[test]
fn cost_without_any_cheapest_is_rejected() {
    let msg =
        reject("GRAPH_TABLE(MATCH p = (a)-[t:T COST t.weight]->{1,3}(b) COLUMNS (path_length(p)))");
    assert!(msg.contains("only meaningful under ANY CHEAPEST"), "{msg}");
}

// -------------------------------------------------------------- inline WHERE

#[test]
fn inline_where_in_a_node_pattern_is_rejected_loudly() {
    let msg = reject("GRAPH_TABLE(MATCH (n:User WHERE n.active = true) COLUMNS (n.id))");
    assert!(msg.contains("inline WHERE"), "{msg}");
    assert!(msg.contains("GRAPH_TABLE WHERE clause"), "{msg}");
}

#[test]
fn inline_where_without_a_label_is_rejected_too() {
    let msg = reject("GRAPH_TABLE(MATCH (n WHERE n.id = 'x') COLUMNS (n.id))");
    assert!(msg.contains("inline WHERE"), "{msg}");
}

#[test]
fn inline_where_in_a_relationship_pattern_is_rejected_loudly() {
    let msg = reject("GRAPH_TABLE(MATCH (a)-[r:k WHERE r.since > 2020]->(b) COLUMNS (a.id))");
    assert!(msg.contains("inline WHERE"), "{msg}");
}

/// The top-level WHERE clause must keep working — the rejection is scoped to
/// the inside of `(...)` and `[...]`.
#[test]
fn top_level_where_is_unaffected() {
    let q = parse_ok("GRAPH_TABLE(MATCH (a:User) WHERE a.active = true COLUMNS (a.id))");
    assert!(q.where_clause.is_some());
}

#[test]
fn a_variable_named_wherever_is_not_an_inline_where() {
    let q = parse_ok("GRAPH_TABLE(MATCH (wherever)-[:k]->(b) COLUMNS (wherever.id))");
    match &first_path(&q).elements[0] {
        PatternElement::Node(n) => assert_eq!(n.variable, Some("wherever".into())),
        other => panic!("expected node, got {other:?}"),
    }
}

// ------------------------------------------------------------ type alternation

/// `-[:a|b]->` must retain BOTH types. Only the first was ever read downstream,
/// which silently dropped every `b` edge; the parser side is pinned here.
#[test]
fn relationship_type_alternation_keeps_every_type() {
    let q = parse_ok("GRAPH_TABLE(MATCH (a)-[r:knows|follows|blocks]->{1,3}(b) COLUMNS (a.id))");
    assert_eq!(
        relationship(&q, 1).types,
        vec![
            "knows".to_string(),
            "follows".to_string(),
            "blocks".to_string()
        ]
    );
}

#[test]
fn type_alternation_survives_a_quantifier_and_a_cost() {
    let q = parse_ok(
        "GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[t:knows|follows COST t.weight]->{1,3}(b) \
         COLUMNS (path_length(p)))",
    );
    assert_eq!(
        relationship(&q, 1).types,
        vec!["knows".to_string(), "follows".to_string()]
    );
}

// ------------------------------------------------------------- error quality

#[test]
fn rejections_carry_a_location() {
    let err = parse_graph_table("GRAPH_TABLE(MATCH SIMPLE p = (a)-[:k]->*(b) COLUMNS (a.id))")
        .unwrap_err();
    assert!(err.line >= 1);
    assert!(err.column >= 1);
    assert!(!err.message.contains("Expected keyword"), "{err}");
}

/// A generic syntax error must not inherit a message from an earlier parse.
#[test]
fn a_later_generic_error_is_not_given_a_stale_diagnostic() {
    let _ = parse_graph_table("GRAPH_TABLE(MATCH SIMPLE p = (a)-[:k]->*(b) COLUMNS (a.id))");
    let err = parse_graph_table("GRAPH_TABLE(MATCH (a) COLUMNS a.id)").unwrap_err();
    assert!(!err.message.contains("SIMPLE"), "{err}");
}

// ----------------------------------------------------- shipped demo content

/// `examples/demo/proteingraph/demo-queries.sql` is user-facing content that
/// people copy. Pin the exact patterns it ships, in both spellings.
#[test]
fn shipped_demo_query_patterns_parse() {
    for sql in [
        "GRAPH_TABLE(MATCH (start:Protein)-[:INTERACTS_WITH]->{2,3}(distant:Protein) \
         WHERE start.path = '/alzheimer-study/proteins/APP' \
         COLUMNS (distant.name AS target_name))",
        "GRAPH_TABLE(MATCH (drug:Drug)-[:TARGETS]->(target:Protein)\
         -[:INTERACTS_WITH]->{1,2}(downstream:Protein) \
         WHERE drug.path = '/alzheimer-study/drugs/ADUCANUMAB' \
         COLUMNS (drug.name AS drug_name))",
        // the one query deliberately left in the deprecated spelling
        "GRAPH_TABLE(MATCH (start:Protein)-[:INTERACTS_WITH*1..4]->(end:Protein) \
         WHERE start.path = '/alzheimer-study/proteins/BACE1' \
         COLUMNS (end.name AS to_protein))",
    ] {
        parse_ok(sql);
    }
}
