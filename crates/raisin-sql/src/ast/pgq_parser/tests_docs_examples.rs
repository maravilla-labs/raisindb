//! Every GRAPH_TABLE example printed in the PUBLIC PGQ reference must parse.
//!
//! Mirrors `raisindb-website/docs/reference/sql/graph/pgq.md`. Docs that show
//! syntax the parser rejects are worse than no docs, and two examples in that
//! page were exactly that before this existed: a `COST` on an anonymous edge,
//! and `COST r.duration_min` — a RaisinDB relation has no arbitrary property
//! map, so an edge cost can only be `r.weight`.
//!
//! When you change an example on that page, change it here too.

use super::parse_graph_table;

fn ok(sql: &str) {
    if let Err(e) = parse_graph_table(sql) {
        panic!("DOC EXAMPLE FAILED TO PARSE:\n  {sql}\n  error: {e}");
    }
}

fn rejected(sql: &str) {
    if parse_graph_table(sql).is_ok() {
        panic!("DOC CLAIMED THIS IS REJECTED, BUT IT PARSED:\n  {sql}");
    }
}

#[test]
fn public_pgq_doc_examples_parse() {
    // Path patterns
    ok("GRAPH_TABLE(MATCH (a:Page)-[:LINKS_TO]->(b)-[:LINKS_TO]->(c) COLUMNS (a.title AS start, c.title AS end))");
    ok("GRAPH_TABLE(MATCH (a:Page)-[:LINKS_TO]->{1,3}(b:Page) COLUMNS (a.title, b.title))");

    // Quantifier table
    ok("GRAPH_TABLE(MATCH (a)-[:LINKS_TO]->{2}(b) COLUMNS (a.title AS start, b.title AS end))");
    ok("GRAPH_TABLE(MATCH (a)-[:LINKS_TO]->{1,3}(b) COLUMNS (a.title, b.title))");
    ok("GRAPH_TABLE(MATCH TRAIL (a)-[:LINKS_TO]->{2,}(b) COLUMNS (a.title, b.title))");

    // Q-SCOPE: the doc says the bare form is an error and the scoped ones are not.
    rejected("GRAPH_TABLE(MATCH (a)-[:LINKS_TO]->*(b) COLUMNS (a.title))");
    ok("GRAPH_TABLE(MATCH ANY SHORTEST p = (a)-[:LINKS_TO]->*(b) COLUMNS (path_length(p)))");
    ok("GRAPH_TABLE(MATCH TRAIL (a)-[:LINKS_TO]->*(b) COLUMNS (a.title))");

    // Deprecated quantifier mapping table
    ok("GRAPH_TABLE(MATCH (a)-[:t*2]->(b) COLUMNS (a.title))");
    ok("GRAPH_TABLE(MATCH (a)-[:t*1..3]->(b) COLUMNS (a.title))");
    ok("GRAPH_TABLE(MATCH (a)-[:t*2..]->(b) COLUMNS (a.title))");
    ok("GRAPH_TABLE(MATCH (a)-[:t*]->(b) COLUMNS (a.title))");

    // Path variables
    ok(
        "GRAPH_TABLE(MATCH p = (a:Page)-[:LINKS_TO]->{1,3}(b:Page) WHERE a.title = 'Home' \
        COLUMNS (path_length(p) AS hops, nodes(p) AS stops))",
    );
    ok("GRAPH_TABLE(MATCH p = ANY SHORTEST (a)-[:t]->{1,3}(b) COLUMNS (path_length(p)))");
    ok("GRAPH_TABLE(MATCH ANY SHORTEST TRAIL p = (a)-[:t]->{1,3}(b) COLUMNS (path_length(p)))");

    // Selectors
    ok(
        "GRAPH_TABLE(MATCH ANY SHORTEST p = (a:Page)-[:LINKS_TO]->{1,6}(b:Page) \
        WHERE a.title = 'Home' AND b.title = 'Pricing' COLUMNS (path_length(p) AS hops))",
    );
    ok(
        "GRAPH_TABLE(MATCH ANY CHEAPEST p = (a:Stop)-[r:ROUTE COST r.weight]->{1,8}(b:Stop) \
        COLUMNS (path_length(p) AS hops))",
    );

    // COST rejections the doc names
    rejected(
        "GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[:ROUTE COST r.weight]->{1,8}(b) COLUMNS (a.id))",
    );
    rejected("GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[r:ROUTE COST r.duration_min]->{1,8}(b) COLUMNS (a.id))");
    rejected("GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[r:ROUTE COST 0]->{1,8}(b) COLUMNS (a.id))");
    rejected(
        "GRAPH_TABLE(MATCH ANY CHEAPEST p = (a)-[r:ROUTE COST s.weight]->{1,8}(b) COLUMNS (a.id))",
    );

    // Restrictors
    ok("GRAPH_TABLE(MATCH WALK (a)-[:t]->{1,3}(b) COLUMNS (a.title))");
    ok("GRAPH_TABLE(MATCH ACYCLIC (a)-[:t]->{1,3}(b) COLUMNS (a.title))");

    // Accessors
    for accessor in [
        "path_length(p)",
        "nodes(p)",
        "edges(p)",
        "path_first(p)",
        "path_last(p)",
        "element_id(p)",
        "is_trail(p)",
        "is_acyclic(p)",
    ] {
        ok(&format!(
            "GRAPH_TABLE(MATCH p = (a)-[:t]->{{1,3}}(b) COLUMNS ({accessor}))"
        ));
    }

    // NOTE: `COLUMNS (p)` deliberately PARSES. A bare path variable is rejected
    // later, at validation, by `path_accessors::path_is_not_selectable`, whose
    // message names the accessors — proven end to end by
    // `raisin-server/tests/all/pgq_path_e2e_test.rs`. Asserting a parse error
    // here would pin the wrong layer.
    ok("GRAPH_TABLE(MATCH p = (a)-[:t]->{1,3}(b) COLUMNS (p))");
}
