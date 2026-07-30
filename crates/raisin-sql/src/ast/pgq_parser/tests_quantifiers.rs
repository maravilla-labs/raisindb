//! Tests for the quantifier migration: the canonical postfix form, the
//! deprecated Cypher-style alias, and rule Q-SCOPE.
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

// ---------------------------------------------------------------- path variable

// ------------------------------------------------------ canonical quantifiers

#[test]
fn standard_range_quantifier() {
    let q = parse_ok("GRAPH_TABLE(MATCH (a)-[:knows]->{1,3}(b) COLUMNS (a.id))");
    let quant = relationship(&q, 1).quantifier.unwrap();
    assert_eq!((quant.min, quant.max), (1, Some(3)));
    assert_eq!(quant.syntax, QuantifierSyntax::Standard);
    assert!(quant.deprecation_note().is_none());
}

#[test]
fn standard_open_ended_quantifier() {
    let q = parse_ok("GRAPH_TABLE(MATCH TRAIL (a)-[:knows]->{2,}(b) COLUMNS (a.id))");
    let quant = relationship(&q, 1).quantifier.unwrap();
    assert_eq!((quant.min, quant.max), (2, None));
    assert!(quant.is_unbounded());
    assert_eq!(quant.syntax, QuantifierSyntax::Standard);
}

#[test]
fn standard_exact_quantifier() {
    let q = parse_ok("GRAPH_TABLE(MATCH (a)-[:knows]->{2}(b) COLUMNS (a.id))");
    let quant = relationship(&q, 1).quantifier.unwrap();
    assert_eq!((quant.min, quant.max), (2, Some(2)));
}

#[test]
fn standard_optional_quantifier() {
    let q = parse_ok("GRAPH_TABLE(MATCH (a)-[:knows]->?(b) COLUMNS (a.id))");
    let quant = relationship(&q, 1).quantifier.unwrap();
    assert_eq!((quant.min, quant.max), (0, Some(1)));
}

/// Standard `*` is `{0,}` — legacy `*` is `{1,}`. They are in different slots
/// so this divergence is never ambiguous, but it must be pinned.
#[test]
fn standard_star_is_zero_or_more_unlike_legacy_star() {
    let q = parse_ok("GRAPH_TABLE(MATCH ACYCLIC (a)-[:knows]->*(b) COLUMNS (a.id))");
    let standard = relationship(&q, 1).quantifier.unwrap();
    assert_eq!((standard.min, standard.max), (0, None));

    let q = parse_ok("GRAPH_TABLE(MATCH (a)-[:knows*]->(b) COLUMNS (a.id))");
    let legacy = relationship(&q, 1).quantifier.unwrap();
    assert_eq!((legacy.min, legacy.max), (1, None));
}

#[test]
fn standard_plus_quantifier() {
    let q = parse_ok("GRAPH_TABLE(MATCH ANY SHORTEST (a)-[:knows]->+(b) COLUMNS (a.id))");
    let quant = relationship(&q, 1).quantifier.unwrap();
    assert_eq!((quant.min, quant.max), (1, None));
}

#[test]
fn quantifier_on_left_and_undirected_arrows() {
    let q = parse_ok("GRAPH_TABLE(MATCH (a)<-[:knows]-{1,2}(b) COLUMNS (a.id))");
    assert_eq!(relationship(&q, 1).direction, Direction::Left);
    assert_eq!(relationship(&q, 1).quantifier.unwrap().max, Some(2));

    let q = parse_ok("GRAPH_TABLE(MATCH (a)-[:knows]-{1,2}(b) COLUMNS (a.id))");
    assert_eq!(relationship(&q, 1).direction, Direction::Any);
    assert_eq!(relationship(&q, 1).quantifier.unwrap().max, Some(2));
}

#[test]
fn undirected_relationship_without_quantifier_still_parses() {
    let q = parse_ok("GRAPH_TABLE(MATCH (a)-[r:KNOWS]-(b) COLUMNS (a.id))");
    assert_eq!(relationship(&q, 1).direction, Direction::Any);
    assert!(relationship(&q, 1).quantifier.is_none());
}

#[test]
fn empty_quantifier_range_is_rejected() {
    let msg = reject("GRAPH_TABLE(MATCH (a)-[:knows]->{3,1}(b) COLUMNS (a.id))");
    assert!(msg.contains("is empty"), "{msg}");
}

#[test]
fn two_quantifiers_on_one_relationship_are_rejected() {
    let msg = reject("GRAPH_TABLE(MATCH (a)-[:knows*1..2]->{1,3}(b) COLUMNS (a.id))");
    assert!(msg.contains("two quantifiers"), "{msg}");
}

// -------------------------------------------------------- legacy quantifiers

#[test]
fn legacy_quantifier_still_parses_and_is_flagged() {
    let q = parse_ok("GRAPH_TABLE(MATCH (a)-[:follows*1..3]->(b) COLUMNS (a.id))");
    let quant = relationship(&q, 1).quantifier.unwrap();
    assert_eq!((quant.min, quant.max), (1, Some(3)));
    assert_eq!(quant.syntax, QuantifierSyntax::LegacyStar);
    let note = quant.deprecation_note().expect("legacy form is deprecated");
    assert!(note.contains("deprecated"), "{note}");
    assert!(note.contains("->{1,3}"), "{note}");
}

/// The legacy form predates rule Q-SCOPE and is exempt from it — it is capped
/// at `DEFAULT_MAX` instead, which is what it has always done.
#[test]
fn legacy_unbounded_quantifier_is_exempt_from_q_scope() {
    let q = parse_ok("GRAPH_TABLE(MATCH (a)-[:FOLLOWS*2..]->(b) COLUMNS (a.id))");
    let quant = relationship(&q, 1).quantifier.unwrap();
    assert_eq!((quant.min, quant.max), (2, None));
    assert_eq!(quant.syntax, QuantifierSyntax::LegacyStar);
    assert_eq!(quant.effective_max(), PathQuantifier::DEFAULT_MAX);
}

// ------------------------------------------------------------- rule Q-SCOPE

#[test]
fn unbounded_standard_quantifier_without_scope_is_rejected() {
    for sql in [
        "GRAPH_TABLE(MATCH (a)-[:k]->*(b) COLUMNS (a.id))",
        "GRAPH_TABLE(MATCH (a)-[:k]->+(b) COLUMNS (a.id))",
        "GRAPH_TABLE(MATCH (a)-[:k]->{2,}(b) COLUMNS (a.id))",
    ] {
        let msg = reject(sql);
        assert!(msg.contains("unbounded quantifier"), "{msg}");
        assert!(
            msg.contains("ANY SHORTEST"),
            "message must name a selector: {msg}"
        );
        assert!(
            msg.contains("TRAIL"),
            "message must name a restrictor: {msg}"
        );
    }
}

#[test]
fn a_selector_alone_satisfies_q_scope() {
    parse_ok("GRAPH_TABLE(MATCH ANY SHORTEST (a)-[:k]->*(b) COLUMNS (a.id))");
}

#[test]
fn a_restrictor_alone_satisfies_q_scope() {
    parse_ok("GRAPH_TABLE(MATCH WALK (a)-[:k]->*(b) COLUMNS (a.id))");
}

#[test]
fn bounded_quantifiers_need_no_scope() {
    parse_ok("GRAPH_TABLE(MATCH (a)-[:k]->{1,3}(b) COLUMNS (a.id))");
    parse_ok("GRAPH_TABLE(MATCH (a)-[:k]->{2}(b) COLUMNS (a.id))");
    parse_ok("GRAPH_TABLE(MATCH (a)-[:k]->?(b) COLUMNS (a.id))");
}
