//! Tests for PGQ GRAPH_TABLE parser

use super::*;
use crate::ast::pgq::*;

#[test]
fn test_is_graph_table() {
    assert!(is_graph_table_expression("SELECT * FROM GRAPH_TABLE(...)"));
    assert!(is_graph_table_expression("graph_table"));
    assert!(!is_graph_table_expression("SELECT * FROM users"));
}

#[test]
fn test_parse_simple_graph_table() {
    let sql = "GRAPH_TABLE(MATCH (a) COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    assert!(result.graph_name.is_none());
    assert_eq!(result.effective_graph_name(), "NODES_GRAPH");
    assert_eq!(result.match_clause.patterns.len(), 1);
    assert_eq!(result.columns_clause.columns.len(), 1);
}

#[test]
fn test_parse_with_graph_name() {
    let sql = "GRAPH_TABLE(my_graph MATCH (a) COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    assert_eq!(result.graph_name, Some("my_graph".to_string()));
}

#[test]
fn test_parse_node_with_label() {
    let sql = "GRAPH_TABLE(MATCH (a:User) COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    if let PatternElement::Node(node) = &pattern.elements[0] {
        assert_eq!(node.variable, Some("a".to_string()));
        assert_eq!(node.labels, vec!["User"]);
    } else {
        panic!("Expected node");
    }
}

#[test]
fn test_parse_relationship() {
    let sql = "GRAPH_TABLE(MATCH (a)-[:follows]->(b) COLUMNS (a.id, b.id))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    assert_eq!(pattern.elements.len(), 3); // node, rel, node

    if let PatternElement::Relationship(rel) = &pattern.elements[1] {
        assert_eq!(rel.types, vec!["follows"]);
        assert_eq!(rel.direction, Direction::Right);
    } else {
        panic!("Expected relationship");
    }
}

#[test]
fn test_parse_variable_length_path() {
    let sql = "GRAPH_TABLE(MATCH (a)-[:follows*1..3]->(b) COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    if let PatternElement::Relationship(rel) = &pattern.elements[1] {
        let q = rel.quantifier.unwrap();
        assert_eq!(q.min, 1);
        assert_eq!(q.max, Some(3));
    } else {
        panic!("Expected relationship");
    }
}

#[test]
fn test_parse_where_clause() {
    let sql = "GRAPH_TABLE(MATCH (a:User) WHERE a.id = 'alice' COLUMNS (a.name))";
    let result = parse_graph_table(sql).unwrap();

    assert!(result.where_clause.is_some());
}

#[test]
fn test_parse_multiple_columns() {
    let sql = "GRAPH_TABLE(MATCH (a) COLUMNS (a.id, a.name AS user_name, a.email))";
    let result = parse_graph_table(sql).unwrap();

    assert_eq!(result.columns_clause.columns.len(), 3);
    assert_eq!(
        result.columns_clause.columns[1].alias,
        Some("user_name".to_string())
    );
}

#[test]
fn test_parse_error_location() {
    let sql = "GRAPH_TABLE(MATCH (a) COLUMNS a.id)"; // Missing parentheses
    let err = parse_graph_table(sql).unwrap_err();

    assert!(err.line >= 1);
    assert!(err.column >= 1);
    assert!(!err.message.is_empty());
}

#[test]
fn test_parse_function_in_columns() {
    let sql = "GRAPH_TABLE(MATCH (a) COLUMNS (degree(a), COUNT(*)))";
    let result = parse_graph_table(sql).unwrap();

    assert_eq!(result.columns_clause.columns.len(), 2);
}

#[test]
fn test_parse_nodes_graph_explicit() {
    let sql =
        "GRAPH_TABLE(NODES_GRAPH MATCH (a:User)-[:FOLLOWS]->(b:User) COLUMNS (a.name, b.name))";
    let result = parse_graph_table(sql).unwrap();

    assert_eq!(result.graph_name, Some("NODES_GRAPH".to_string()));
    assert_eq!(result.effective_graph_name(), "NODES_GRAPH");
    assert_eq!(result.match_clause.patterns[0].elements.len(), 3);
}

#[test]
fn test_parse_multi_label_node() {
    let sql = "GRAPH_TABLE(MATCH (a:User|Admin) COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    if let PatternElement::Node(node) = &pattern.elements[0] {
        assert_eq!(node.labels, vec!["User", "Admin"]);
    } else {
        panic!("Expected node");
    }
}

#[test]
fn test_parse_bidirectional_relationship() {
    let sql = "GRAPH_TABLE(MATCH (a)-[r:KNOWS]-(b) COLUMNS (a.id, r.since, b.id))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    if let PatternElement::Relationship(rel) = &pattern.elements[1] {
        assert_eq!(rel.variable, Some("r".to_string()));
        assert_eq!(rel.types, vec!["KNOWS"]);
        assert_eq!(rel.direction, Direction::Any);
    } else {
        panic!("Expected relationship");
    }
}

#[test]
fn test_parse_single_node_pattern() {
    let sql = "GRAPH_TABLE(MATCH (n) COLUMNS (n))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    assert_eq!(pattern.elements.len(), 1);

    if let PatternElement::Node(node) = &pattern.elements[0] {
        assert_eq!(node.variable, Some("n".to_string()));
        assert!(node.labels.is_empty());
    } else {
        panic!("Expected node");
    }

    assert_eq!(result.columns_clause.columns.len(), 1);
    let col = &result.columns_clause.columns[0];

    if let Expr::PropertyAccess {
        variable,
        properties,
        ..
    } = &col.expr
    {
        assert_eq!(variable, "n");
        assert!(
            properties.is_empty(),
            "Bare node reference should have empty properties"
        );
    } else {
        panic!(
            "Expected PropertyAccess for bare node reference, got {:?}",
            col.expr
        );
    }
}

#[test]
fn test_parse_single_node_with_label() {
    let sql = "GRAPH_TABLE(MATCH (n:Article) COLUMNS (n.id, n.properties))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    assert_eq!(pattern.elements.len(), 1);

    if let PatternElement::Node(node) = &pattern.elements[0] {
        assert_eq!(node.variable, Some("n".to_string()));
        assert_eq!(node.labels, vec!["Article"]);
    } else {
        panic!("Expected node");
    }
}

#[test]
fn test_parse_left_direction() {
    let sql = "GRAPH_TABLE(MATCH (a)<-[:PARENT]-(b) COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    if let PatternElement::Relationship(rel) = &pattern.elements[1] {
        assert_eq!(rel.direction, Direction::Left);
    } else {
        panic!("Expected relationship");
    }
}

#[test]
fn test_parse_complex_where() {
    let sql = "GRAPH_TABLE(MATCH (a:User) WHERE a.active = true AND a.age > 18 COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    assert!(result.where_clause.is_some());
    if let Some(where_clause) = &result.where_clause {
        if let Expr::BinaryOp { op, .. } = &where_clause.expression {
            assert_eq!(*op, BinaryOperator::And);
        } else {
            panic!("Expected binary AND, got {:?}", where_clause.expression);
        }
    }
}

#[test]
fn test_parse_variable_length_star_only() {
    let sql = "GRAPH_TABLE(MATCH (a)-[:FOLLOWS*]->(b) COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    if let PatternElement::Relationship(rel) = &pattern.elements[1] {
        let q = rel.quantifier.unwrap();
        assert_eq!(q.min, 1); // default min
        assert_eq!(q.max, None); // unbounded
        assert_eq!(q.effective_max(), 10); // effective max from default
    } else {
        panic!("Expected relationship");
    }
}

#[test]
fn test_parse_exact_hops() {
    let sql = "GRAPH_TABLE(MATCH (a)-[:FOLLOWS*3]->(b) COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    if let PatternElement::Relationship(rel) = &pattern.elements[1] {
        let q = rel.quantifier.unwrap();
        assert_eq!(q.min, 3);
        assert_eq!(q.max, Some(3));
    } else {
        panic!("Expected relationship");
    }
}

#[test]
fn test_parse_unbounded_max() {
    let sql = "GRAPH_TABLE(MATCH (a)-[:FOLLOWS*2..]->(b) COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    if let PatternElement::Relationship(rel) = &pattern.elements[1] {
        let q = rel.quantifier.unwrap();
        assert_eq!(q.min, 2);
        assert_eq!(q.max, None); // unbounded max
        assert_eq!(q.effective_max(), 10); // effective max from default
    } else {
        panic!("Expected relationship");
    }
}

#[test]
fn test_detect_graph_table_in_select() {
    let full_sql = "SELECT * FROM GRAPH_TABLE(NODES_GRAPH MATCH (a:User) COLUMNS (a.id, a.name)) AS g WHERE g.name LIKE '%alice%'";

    assert!(is_graph_table_expression(full_sql));

    let inner = "GRAPH_TABLE(NODES_GRAPH MATCH (a:User) COLUMNS (a.id, a.name))";
    let result = parse_graph_table(inner).unwrap();
    assert_eq!(result.graph_name, Some("NODES_GRAPH".to_string()));
    assert_eq!(result.columns_clause.columns.len(), 2);
}

#[test]
fn test_find_graph_tables() {
    let sql =
        "SELECT * FROM GRAPH_TABLE(MATCH (a:User) COLUMNS (a.id)) AS users WHERE users.id = '123'";

    let found = find_graph_tables(sql);
    assert_eq!(found.len(), 1);

    let (start, end, result) = &found[0];
    assert!(result.is_ok());
    let query = result.as_ref().unwrap();
    assert_eq!(query.effective_graph_name(), "NODES_GRAPH");

    let extracted = &sql[*start..*end];
    assert!(extracted.starts_with("GRAPH_TABLE("));
    assert!(extracted.ends_with(")"));
}

#[test]
fn test_find_multiple_graph_tables() {
    let sql = r#"
            SELECT u.id, f.name
            FROM GRAPH_TABLE(MATCH (a:User) COLUMNS (a.id)) AS u,
                 GRAPH_TABLE(MATCH (b:Friend) COLUMNS (b.name)) AS f
        "#;

    let found = find_graph_tables(sql);
    assert_eq!(found.len(), 2);
    assert!(found[0].2.is_ok());
    assert!(found[1].2.is_ok());
}

#[test]
fn test_extract_graph_table_arg_quoted() {
    let arg = "'MATCH (a:User) COLUMNS (a.id)'";
    let extracted = extract_graph_table_arg(arg).unwrap();
    assert_eq!(extracted, "MATCH (a:User) COLUMNS (a.id)");
}

#[test]
fn test_extract_graph_table_arg_unquoted() {
    let arg = "MATCH (a:User) COLUMNS (a.id)";
    let extracted = extract_graph_table_arg(arg).unwrap();
    assert_eq!(extracted, "MATCH (a:User) COLUMNS (a.id)");
}

#[test]
fn test_preprocess_graph_tables_simple() {
    let sql = "SELECT * FROM GRAPH_TABLE(MATCH (a:User) COLUMNS (a.id))";
    let processed = preprocess_graph_tables(sql);
    assert!(processed.starts_with("SELECT * FROM GRAPH_TABLE('GRAPH_TABLE("));
    assert!(processed.contains("MATCH (a:User) COLUMNS (a.id)"));
    assert!(processed.ends_with(")')"));
}

#[test]
fn test_preprocess_graph_tables_with_quotes() {
    let sql = "SELECT * FROM GRAPH_TABLE(MATCH (a:User) WHERE a.name = 'alice' COLUMNS (a.id))";
    let processed = preprocess_graph_tables(sql);
    assert!(processed.contains("''alice''"));
}

#[test]
fn test_preprocess_graph_tables_multiple() {
    let sql = "SELECT * FROM GRAPH_TABLE(MATCH (a) COLUMNS (a.id)) AS t1, GRAPH_TABLE(MATCH (b) COLUMNS (b.id)) AS t2";
    let processed = preprocess_graph_tables(sql);
    assert_eq!(processed.matches("GRAPH_TABLE('").count(), 2);
}

#[test]
fn test_find_graph_table_with_nested_parens() {
    let sql = "SELECT * FROM GRAPH_TABLE(MATCH (a:User) WHERE (a.age > 18 AND (a.active = true)) COLUMNS (a.id, a.name))";

    let found = find_graph_tables(sql);
    assert_eq!(found.len(), 1);
    assert!(found[0].2.is_ok());
}

#[test]
fn test_parse_chained_directions() {
    let sql = "GRAPH_TABLE(MATCH (a)-[:X]->(b)<-[:Y]-(c) COLUMNS (a.id, b.id, c.id))";
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    assert_eq!(pattern.elements.len(), 5); // 3 nodes + 2 relationships

    if let PatternElement::Relationship(rel) = &pattern.elements[1] {
        assert_eq!(rel.direction, Direction::Right);
        assert_eq!(rel.types, vec!["X".to_string()]);
    } else {
        panic!("Expected relationship at index 1");
    }

    if let PatternElement::Relationship(rel) = &pattern.elements[3] {
        assert_eq!(rel.direction, Direction::Left);
        assert_eq!(rel.types, vec!["Y".to_string()]);
    } else {
        panic!("Expected relationship at index 3");
    }
}

#[test]
fn test_parse_chained_directions_with_labels() {
    let sql = r#"GRAPH_TABLE(MATCH (this:Article)-[:`tagged-with`]->(tag:Tag)<-[:`tagged-with`]-(other:Article) COLUMNS (this.id, tag.name, other.id))"#;
    let result = parse_graph_table(sql).unwrap();

    let pattern = &result.match_clause.patterns[0];
    assert_eq!(pattern.elements.len(), 5); // 3 nodes + 2 relationships

    if let PatternElement::Node(node) = &pattern.elements[0] {
        assert_eq!(node.variable, Some("this".to_string()));
        assert_eq!(node.labels, vec!["Article".to_string()]);
    }

    if let PatternElement::Node(node) = &pattern.elements[2] {
        assert_eq!(node.variable, Some("tag".to_string()));
        assert_eq!(node.labels, vec!["Tag".to_string()]);
    }

    if let PatternElement::Node(node) = &pattern.elements[4] {
        assert_eq!(node.variable, Some("other".to_string()));
        assert_eq!(node.labels, vec!["Article".to_string()]);
    }
}

#[test]
fn test_parse_json_access_double_arrow() {
    let sql = "GRAPH_TABLE(MATCH (a) WHERE a.properties->>'name' = 'Alice' COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    assert!(result.where_clause.is_some());
    if let Some(where_clause) = result.where_clause {
        if let Expr::BinaryOp { left, op, .. } = &where_clause.expression {
            assert_eq!(*op, BinaryOperator::Eq);
            if let Expr::JsonAccess { key, as_text, .. } = left.as_ref() {
                assert_eq!(key, "name");
                assert!(*as_text); // ->> returns text
            } else {
                panic!("Expected JsonAccess, got {:?}", left);
            }
        } else {
            panic!("Expected BinaryOp");
        }
    }
}

#[test]
fn test_parse_json_access_single_arrow() {
    let sql = "GRAPH_TABLE(MATCH (a) COLUMNS (a.properties->'address' AS addr))";
    let result = parse_graph_table(sql).unwrap();

    assert_eq!(result.columns_clause.columns.len(), 1);
    let col = &result.columns_clause.columns[0];
    if let Expr::JsonAccess { key, as_text, .. } = &col.expr {
        assert_eq!(key, "address");
        assert!(!as_text); // -> returns JSON, not text
    } else {
        panic!("Expected JsonAccess, got {:?}", col.expr);
    }
    assert_eq!(col.alias, Some("addr".to_string()));
}

#[test]
fn test_parse_chained_json_access() {
    let sql =
        "GRAPH_TABLE(MATCH (a) WHERE a.properties->'address'->>'city' = 'NYC' COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    assert!(result.where_clause.is_some());
    if let Some(where_clause) = result.where_clause {
        if let Expr::BinaryOp { left, .. } = &where_clause.expression {
            if let Expr::JsonAccess {
                expr, key, as_text, ..
            } = left.as_ref()
            {
                assert_eq!(key, "city");
                assert!(*as_text); // ->>
                if let Expr::JsonAccess {
                    key: inner_key,
                    as_text: inner_as_text,
                    ..
                } = expr.as_ref()
                {
                    assert_eq!(inner_key, "address");
                    assert!(!inner_as_text); // ->
                } else {
                    panic!("Expected inner JsonAccess");
                }
            } else {
                panic!("Expected JsonAccess");
            }
        }
    }
}

#[test]
fn test_parse_jsonpath_access() {
    let sql = "GRAPH_TABLE(MATCH (a) WHERE $.friend.properties.email = 'test@example.com' COLUMNS (a.id))";
    let result = parse_graph_table(sql).unwrap();

    assert!(result.where_clause.is_some());
    if let Some(where_clause) = result.where_clause {
        if let Expr::BinaryOp { left, .. } = &where_clause.expression {
            if let Expr::JsonPathAccess { variable, path, .. } = left.as_ref() {
                assert_eq!(variable, "friend");
                assert_eq!(path, &vec!["properties".to_string(), "email".to_string()]);
            } else {
                panic!("Expected JsonPathAccess, got {:?}", left);
            }
        }
    }
}

#[test]
fn test_parse_jsonpath_in_columns() {
    let sql = "GRAPH_TABLE(MATCH (a) COLUMNS ($.friend.properties.email AS friend_email))";
    let result = parse_graph_table(sql).unwrap();

    assert_eq!(result.columns_clause.columns.len(), 1);
    let col = &result.columns_clause.columns[0];
    if let Expr::JsonPathAccess { variable, path, .. } = &col.expr {
        assert_eq!(variable, "friend");
        assert_eq!(path, &vec!["properties".to_string(), "email".to_string()]);
        assert_eq!(col.alias, Some("friend_email".to_string()));
    } else {
        panic!("Expected JsonPathAccess in column");
    }
}
