// SPDX-License-Identifier: BSL-1.1

//! Integration tests for Cypher parser
//!
//! Tests the parser with real-world Cypher queries

use raisin_cypher_parser::ast::{RemoveItem, SetItem};
use raisin_cypher_parser::*;

#[test]
fn test_simple_match_return() {
    let query = "MATCH (n:Person) RETURN n";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.clauses.len(), 2);

    match &query.clauses[0] {
        Clause::Match { optional, pattern } => {
            assert!(!optional);
            assert_eq!(pattern.patterns.len(), 1);
        }
        _ => panic!("Expected MATCH clause"),
    }

    match &query.clauses[1] {
        Clause::Return { items, .. } => {
            assert_eq!(items.len(), 1);
        }
        _ => panic!("Expected RETURN clause"),
    }
}

#[test]
fn test_match_with_relationship() {
    let query = "MATCH (a)-[:KNOWS]->(b) WHERE a.name = 'Alice' RETURN b.name";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.clauses.len(), 3); // MATCH, WHERE, RETURN

    match &query.clauses[0] {
        Clause::Match { pattern, .. } => {
            assert_eq!(pattern.patterns.len(), 1);
        }
        _ => panic!("Expected MATCH clause"),
    }

    match &query.clauses[1] {
        Clause::Where { .. } => {}
        _ => panic!("Expected WHERE clause"),
    }
}

#[test]
fn test_create_with_properties() {
    let query = "CREATE (n:Person {name: 'Bob', age: 30})";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.clauses.len(), 1);

    match &query.clauses[0] {
        Clause::Create { pattern } => {
            assert_eq!(pattern.patterns.len(), 1);
            let path = &pattern.patterns[0];
            assert_eq!(path.elements.len(), 1);

            match &path.elements[0] {
                PatternElement::Node(node) => {
                    assert_eq!(node.labels, vec!["Person"]);
                    assert!(node.properties.is_some());
                    let props = node.properties.as_ref().unwrap();
                    assert_eq!(props.len(), 2);
                }
                _ => panic!("Expected node pattern"),
            }
        }
        _ => panic!("Expected CREATE clause"),
    }
}

#[test]
fn test_complex_match_create_return() {
    // This is the user's example query
    let query = r#"
        MATCH (a:Person), (b:Person)
        WHERE a.name = 'Node A' AND b.name = 'Node B'
        CREATE (a)-[e:RELTYPE {name: a.name + '<->' + b.name}]->(b)
        RETURN e
    "#;

    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.clauses.len(), 4); // MATCH, WHERE, CREATE, RETURN

    // Check MATCH clause
    match &query.clauses[0] {
        Clause::Match { pattern, .. } => {
            assert_eq!(pattern.patterns.len(), 2); // Two patterns: (a:Person), (b:Person)
        }
        _ => panic!("Expected MATCH clause"),
    }

    // Check WHERE clause
    match &query.clauses[1] {
        Clause::Where { .. } => {}
        _ => panic!("Expected WHERE clause"),
    }

    // Check CREATE clause
    match &query.clauses[2] {
        Clause::Create { pattern } => {
            assert_eq!(pattern.patterns.len(), 1);
        }
        _ => panic!("Expected CREATE clause"),
    }

    // Check RETURN clause
    match &query.clauses[3] {
        Clause::Return { items, .. } => {
            assert_eq!(items.len(), 1);
        }
        _ => panic!("Expected RETURN clause"),
    }
}

#[test]
fn test_optional_match() {
    let query = "OPTIONAL MATCH (n:Person) RETURN n";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[0] {
        Clause::Match { optional, .. } => {
            assert!(optional);
        }
        _ => panic!("Expected MATCH clause"),
    }
}

#[test]
fn test_return_with_order_by() {
    let query = "MATCH (n:Person) RETURN n.name, n.age ORDER BY n.age DESC";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[1] {
        Clause::Return {
            items, order_by, ..
        } => {
            assert_eq!(items.len(), 2);
            assert_eq!(order_by.len(), 1);
            assert_eq!(order_by[0].order, Order::Desc);
        }
        _ => panic!("Expected RETURN clause"),
    }
}

#[test]
fn test_return_with_skip_limit() {
    let query = "MATCH (n:Person) RETURN n SKIP 10 LIMIT 20";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[1] {
        Clause::Return { skip, limit, .. } => {
            assert!(skip.is_some());
            assert!(limit.is_some());
        }
        _ => panic!("Expected RETURN clause"),
    }
}

#[test]
fn test_return_with_alias() {
    let query = "MATCH (n:Person) RETURN n.name AS name, n.age AS age";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[1] {
        Clause::Return { items, .. } => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].alias.as_deref(), Some("name"));
            assert_eq!(items[1].alias.as_deref(), Some("age"));
        }
        _ => panic!("Expected RETURN clause"),
    }
}

#[test]
fn test_return_distinct() {
    let query = "MATCH (n:Person) RETURN DISTINCT n.name";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[1] {
        Clause::Return { distinct, .. } => {
            assert!(distinct);
        }
        _ => panic!("Expected RETURN clause"),
    }
}

#[test]
fn test_variable_length_relationship() {
    let query = "MATCH (a)-[:KNOWS*1..3]->(b) RETURN a, b";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[0] {
        Clause::Match { pattern, .. } => {
            let path = &pattern.patterns[0];
            match &path.elements[1] {
                PatternElement::Relationship(rel) => {
                    assert!(rel.range.is_some());
                    let range = rel.range.as_ref().unwrap();
                    assert_eq!(range.min, Some(1));
                    assert_eq!(range.max, Some(3));
                }
                _ => panic!("Expected relationship pattern"),
            }
        }
        _ => panic!("Expected MATCH clause"),
    }
}

#[test]
fn test_multiple_labels() {
    let query = "MATCH (n:Person:Employee) RETURN n";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[0] {
        Clause::Match { pattern, .. } => {
            let path = &pattern.patterns[0];
            match &path.elements[0] {
                PatternElement::Node(node) => {
                    assert_eq!(node.labels, vec!["Person", "Employee"]);
                }
                _ => panic!("Expected node pattern"),
            }
        }
        _ => panic!("Expected MATCH clause"),
    }
}

#[test]
fn test_bidirectional_relationship() {
    let query = "MATCH (a)<-[:KNOWS]->(b) RETURN a, b";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[0] {
        Clause::Match { pattern, .. } => {
            let path = &pattern.patterns[0];
            match &path.elements[1] {
                PatternElement::Relationship(rel) => {
                    assert_eq!(rel.direction, Direction::Both);
                }
                _ => panic!("Expected relationship pattern"),
            }
        }
        _ => panic!("Expected MATCH clause"),
    }
}

#[test]
fn test_undirected_relationship() {
    let query = "MATCH (a)-[:KNOWS]-(b) RETURN a, b";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[0] {
        Clause::Match { pattern, .. } => {
            let path = &pattern.patterns[0];
            match &path.elements[1] {
                PatternElement::Relationship(rel) => {
                    assert_eq!(rel.direction, Direction::None);
                }
                _ => panic!("Expected relationship pattern"),
            }
        }
        _ => panic!("Expected MATCH clause"),
    }
}

#[test]
fn test_with_clause() {
    let query = "MATCH (n:Person) WITH n, n.age AS age WHERE age > 18 RETURN n";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    assert_eq!(query.clauses.len(), 3);

    match &query.clauses[1] {
        Clause::With { items, .. } => {
            assert_eq!(items.len(), 2);
        }
        _ => panic!("Expected WITH clause"),
    }
}

#[test]
fn test_delete_clause() {
    let query = "MATCH (n:Person) DELETE n";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[1] {
        Clause::Delete { detach, items } => {
            assert!(!detach);
            assert_eq!(items.len(), 1);
        }
        _ => panic!("Expected DELETE clause"),
    }
}

#[test]
fn test_detach_delete_clause() {
    let query = "MATCH (n:Person) DETACH DELETE n";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[1] {
        Clause::Delete { detach, .. } => {
            assert!(detach);
        }
        _ => panic!("Expected DETACH DELETE clause"),
    }
}

#[test]
fn test_set_property() {
    let query = "MATCH (n:Person) SET n.age = 30";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[1] {
        Clause::Set { items } => {
            assert_eq!(items.len(), 1);
            match &items[0] {
                SetItem::Property {
                    variable, property, ..
                } => {
                    assert_eq!(variable, "n");
                    assert_eq!(property, "age");
                }
                _ => panic!("Expected property set item"),
            }
        }
        _ => panic!("Expected SET clause"),
    }
}

#[test]
fn test_merge_clause() {
    let query = "MERGE (n:Person {name: 'Bob'})";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[0] {
        Clause::Merge { pattern } => {
            assert_eq!(pattern.patterns.len(), 1);
        }
        _ => panic!("Expected MERGE clause"),
    }
}

#[test]
fn test_unwind_clause() {
    let query = "UNWIND [1, 2, 3] AS x RETURN x";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[0] {
        Clause::Unwind { alias, .. } => {
            assert_eq!(alias, "x");
        }
        _ => panic!("Expected UNWIND clause"),
    }
}

#[test]
fn test_function_calls() {
    let query = "MATCH (n) RETURN count(n), sum(n.age), avg(n.age)";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[1] {
        Clause::Return { items, .. } => {
            assert_eq!(items.len(), 3);
            for item in items {
                match &item.expr {
                    Expr::FunctionCall { .. } => {}
                    _ => panic!("Expected function call"),
                }
            }
        }
        _ => panic!("Expected RETURN clause"),
    }
}

#[test]
fn test_case_expression() {
    let query = "MATCH (n) RETURN CASE WHEN n.age > 18 THEN 'adult' ELSE 'minor' END";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    match &query.clauses[1] {
        Clause::Return { items, .. } => match &items[0].expr {
            Expr::Case { .. } => {}
            _ => panic!("Expected CASE expression"),
        },
        _ => panic!("Expected RETURN clause"),
    }
}

#[test]
fn test_parameter_reference() {
    let query = "MATCH (n:Person {name: $name}) RETURN n";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
}

#[test]
fn test_list_expression() {
    let query = "RETURN [1, 2, 3, 4, 5] AS numbers";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
}

#[test]
fn test_map_expression() {
    let query = "RETURN {name: 'Alice', age: 30} AS person";
    let result = parse_query(query);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
}

#[test]
fn test_error_on_invalid_syntax() {
    let query = "MATCH (n RETURN n"; // Missing closing paren
    let result = parse_query(query);
    assert!(result.is_err());

    let err = result.unwrap_err();
    // Error should have position information
    match err {
        ParseError::SyntaxError { line, column, .. } => {
            assert!(line > 0);
            assert!(column > 0);
        }
        _ => {}
    }
}
