//! Regressions found when the analyzer unit suite was found failing wholesale.

use crate::analyzer::{AnalysisError, AnalyzedStatement, Analyzer, StaticCatalog};

/// The catalog-less analyzer (only the built-in `default` workspace) keeps the
/// legacy `nodes` table; a catalog that knows a real workspace must not let
/// `FROM nodes` shadow it.
#[test]
fn legacy_nodes_table_only_without_real_workspaces() {
    assert!(Analyzer::new().analyze("SELECT id FROM nodes").is_ok());

    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("pages".to_string());
    let analyzer = Analyzer::with_catalog(Box::new(catalog));
    match analyzer.analyze("SELECT id FROM nodes") {
        Err(AnalysisError::TableNotFound(t)) => assert_eq!(t, "nodes"),
        other => panic!("expected TableNotFound(nodes), got {:?}", other.map(|_| ())),
    }
    assert!(analyzer.analyze("SELECT id FROM pages").is_ok());
}

/// The aggregate name is upper-cased before its modifiers are parsed, so the
/// DESC marker is `D`: it used to be compared against `d` and every
/// `ARRAY_AGG(x ORDER BY y DESC)` silently sorted ascending.
#[test]
fn array_agg_inner_order_by_desc_survives_analysis() {
    let analyzer = Analyzer::new();
    let stmt = analyzer
        .analyze("SELECT ARRAY_AGG(name ORDER BY name DESC, id ASC) FROM nodes")
        .expect("analyzes");
    let AnalyzedStatement::Query(query) = stmt else {
        panic!("expected a query");
    };
    let agg = query.aggregates.first().expect("one aggregate");
    let dirs: Vec<bool> = agg.order_by.iter().map(|(_, desc)| *desc).collect();
    assert_eq!(dirs, vec![true, false]);
}
