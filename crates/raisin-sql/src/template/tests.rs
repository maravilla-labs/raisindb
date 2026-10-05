use super::*;
use crate::analyzer::{AnalyzedStatement, Analyzer, DataType, Literal, StaticCatalog};
use crate::logical_plan::PlanBuilder;
use crate::optimizer::Optimizer;
use std::sync::Arc;

fn catalog() -> Arc<dyn crate::analyzer::Catalog> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("ws".to_string());
    Arc::new(catalog)
}

fn types(values: &[Literal]) -> Arc<[DataType]> {
    values.iter().map(Literal::data_type).collect()
}

/// Template → bind → rebind equals analyzing and optimizing the text.
fn bound_equals_text(template: &str, rendered: &[&str]) {
    let catalog = catalog();
    let values: Vec<Literal> = rendered.iter().map(|r| param_literal(r).unwrap()).collect();
    let analyzer = Analyzer::with_catalog_arc(catalog.clone());
    let mut stmt = analyzer.analyze_template(template, types(&values)).unwrap();
    check(&stmt).unwrap_or_else(|e| panic!("{template}: {e}"));
    let optimizer = Optimizer::default();
    let mut plan = optimizer.optimize(PlanBuilder::new(catalog.as_ref()).build(&stmt).unwrap());
    bind_statement(&mut stmt, &values).unwrap();
    bind_plan(&mut plan, &values).unwrap();
    let plan = optimizer.rebind(plan);

    let mut text = template.to_string();
    for (i, r) in rendered.iter().enumerate().rev() {
        text = text.replace(&format!("${}", i + 1), r);
    }
    let expected = analyzer.analyze(&text).unwrap();
    let expected_plan =
        optimizer.optimize(PlanBuilder::new(catalog.as_ref()).build(&expected).unwrap());
    let (AnalyzedStatement::Query(got), AnalyzedStatement::Query(want)) = (&stmt, &expected) else {
        panic!("{template}: not a query");
    };
    assert_eq!(got, want, "{template}");
    assert_eq!(plan, expected_plan, "{template}");
}

#[test]
fn bound_templates_equal_their_substituted_text() {
    bound_equals_text("SELECT * FROM ws WHERE path = $1", &["'/a/b'"]);
    bound_equals_text("SELECT id FROM ws WHERE id = $1", &["'n1'"]);
    bound_equals_text(
        "SELECT * FROM ws WHERE properties->>'slug'::String = $1 LIMIT 1",
        &["'s-1'"],
    );
    bound_equals_text("SELECT * FROM ws WHERE CHILD_OF($1)", &["'/menu'"]);
    bound_equals_text("SELECT * FROM ws WHERE DESCENDANT_OF($1)", &["'/menu'"]);
    bound_equals_text(
        "SELECT * FROM ws WHERE locale = $1 AND path = $2",
        &["'fr'", "'/a'"],
    );
    bound_equals_text(
        "SELECT id FROM ws WHERE node_type = $1 AND properties->>'n'::String > $2",
        &["'t:Doc'", "'5'"],
    );
    bound_equals_text("SELECT id FROM ws WHERE id IN ($1, $2)", &["'a'", "'b'"]);
}

#[test]
fn value_steered_positions_are_refused() {
    let catalog = catalog();
    let analyzer = Analyzer::with_catalog_arc(catalog);
    for (sql, value) in [
        ("SELECT * FROM ws WHERE __branch = $1", "'dev'"),
        ("SELECT * FROM ws WHERE __revision = $1", "5"),
        ("SELECT $1 AS x FROM ws", "'a'"),
        ("SELECT * FROM ws WHERE name LIKE $1", "'a%'"),
        ("SELECT * FROM ws WHERE DEPTH(path) = DEPTH($1)", "'/a/b'"),
        ("SELECT * FROM ws WHERE properties @> $1", "'{}'"),
    ] {
        let values = vec![param_literal(value).unwrap()];
        match analyzer.analyze_template(sql, types(&values)) {
            Ok(stmt) => assert!(check(&stmt).is_err(), "{sql} admitted"),
            Err(_) => {} // refused at analysis: planned per value
        }
    }
}

#[test]
fn param_literal_is_the_analyzer_rule() {
    assert_eq!(param_literal("'it''s'"), Some(Literal::Text("it's".into())));
    assert_eq!(param_literal("42"), Some(Literal::Int(42)));
    assert_eq!(
        param_literal("4294967296"),
        Some(Literal::BigInt(4294967296))
    );
    assert_eq!(param_literal("1.5"), Some(Literal::Double(1.5)));
    assert_eq!(param_literal("NULL"), Some(Literal::Null));
    assert_eq!(param_literal("true"), Some(Literal::Boolean(true)));
    // A negative number is a unary minus in the text, not a literal.
    assert_eq!(param_literal("-5"), None);
    assert_eq!(param_literal("ARRAY['a']"), None);
}

#[test]
fn placeholders_inside_strings_are_not_tokens() {
    assert!(placeholders_are_tokens("SELECT * FROM ws WHERE id = $1"));
    assert!(!placeholders_are_tokens(
        "SELECT * FROM ws WHERE name = 'cost $1'"
    ));
    assert!(!placeholders_are_tokens("SELECT * FROM ws WHERE a$1 = 2"));
}
