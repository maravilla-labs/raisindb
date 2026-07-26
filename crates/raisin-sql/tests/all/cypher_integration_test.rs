//! Integration tests for Cypher table-valued function

use raisin_cypher_parser::parse_query;

#[test]
fn test_cypher_parser_integration() {
    // Test that we can parse Cypher queries
    let query = "MATCH (n:Person) RETURN n";
    let result = parse_query(query);
    assert!(result.is_ok(), "Should parse simple Cypher query");

    let query = result.unwrap();
    assert_eq!(query.clauses.len(), 2); // MATCH and RETURN
}

#[test]
fn test_complex_cypher_query() {
    // This is the user's example query
    let query = r#"
        MATCH (a:Person), (b:Person)
        WHERE a.name = 'Node A' AND b.name = 'Node B'
        CREATE (a)-[e:RELTYPE {name: a.name + '<->' + b.name}]->(b)
        RETURN e
    "#;

    let result = parse_query(query);
    assert!(result.is_ok(), "Should parse user's example query");

    let query = result.unwrap();
    // Should have: MATCH, WHERE, CREATE, RETURN
    assert_eq!(query.clauses.len(), 4);
}

#[test]
fn test_cypher_function_validation() {
    // Test that the cypher() function is recognized
    use raisin_sql::ast::functions::RaisinFunction;

    let func = RaisinFunction::from_name("CYPHER");
    assert!(func.is_some(), "Should recognize CYPHER function");

    let func = func.unwrap();
    assert!(func.is_table_valued(), "CYPHER should be table-valued");
    assert!(func.allows_arg_count(1), "CYPHER accepts 1 argument");
    assert!(
        func.allows_arg_count(2),
        "CYPHER accepts optional params argument"
    );
}

#[test]
fn test_parse_outgoing_relationship() {
    // Test parsing outgoing relationship pattern: (a)-[:KNOWS]->(b)
    let query = "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse outgoing relationship query: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert_eq!(query.clauses.len(), 2); // MATCH and RETURN

    // Verify MATCH clause
    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        assert_eq!(pattern.patterns.len(), 1, "Should have one path pattern");

        let path = &pattern.patterns[0];
        assert_eq!(
            path.elements.len(),
            3,
            "Should have 3 elements: node-rel-node"
        );

        // Verify it's node-relationship-node
        assert!(matches!(
            &path.elements[0],
            raisin_cypher_parser::PatternElement::Node(_)
        ));
        assert!(matches!(
            &path.elements[1],
            raisin_cypher_parser::PatternElement::Relationship(_)
        ));
        assert!(matches!(
            &path.elements[2],
            raisin_cypher_parser::PatternElement::Node(_)
        ));

        // Check relationship direction
        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert_eq!(
                rel.direction,
                raisin_cypher_parser::Direction::Right,
                "Should be outgoing (->)"
            );
            assert_eq!(
                rel.types,
                vec!["KNOWS".to_string()],
                "Should have KNOWS type"
            );
        }
    } else {
        panic!("First clause should be MATCH");
    }
}

#[test]
fn test_parse_incoming_relationship() {
    // Test parsing incoming relationship pattern: (a)<-[:CREATED]-(b)
    let query = "MATCH (a:Page)<-[:CREATED]-(b:User) RETURN a, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse incoming relationship query: {:?}",
        result.err()
    );

    let query = result.unwrap();

    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];

        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert_eq!(
                rel.direction,
                raisin_cypher_parser::Direction::Left,
                "Should be incoming (<-)"
            );
            assert_eq!(rel.types, vec!["CREATED".to_string()]);
        }
    }
}

#[test]
fn test_parse_bidirectional_relationship() {
    // Test parsing bidirectional relationship pattern: (a)-[:LINK]-(b)
    let query = "MATCH (a)-[:LINK]-(b) RETURN a, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse bidirectional relationship query: {:?}",
        result.err()
    );

    let query = result.unwrap();

    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];

        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert_eq!(
                rel.direction,
                raisin_cypher_parser::Direction::None,
                "Should be undirected (-)"
            );
        }
    }
}

#[test]
fn test_parse_relationship_with_variable() {
    // Test parsing relationship with variable binding: (a)-[r:KNOWS]->(b)
    let query = "MATCH (a)-[r:KNOWS]->(b) RETURN a, r, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse relationship with variable: {:?}",
        result.err()
    );

    let query = result.unwrap();

    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];

        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert_eq!(
                rel.variable,
                Some("r".to_string()),
                "Should bind to variable 'r'"
            );
            assert_eq!(rel.types, vec!["KNOWS".to_string()]);
        }
    }
}

#[test]
fn test_parse_multi_type_relationship() {
    // Test parsing relationship with multiple types: (a)-[:LINK|REFERENCE]->(b)
    let query = "MATCH (a)-[:LINK|REFERENCE]->(b) RETURN a, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse multi-type relationship: {:?}",
        result.err()
    );

    let query = result.unwrap();

    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];

        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert_eq!(rel.types.len(), 2, "Should have 2 types");
            assert!(rel.types.contains(&"LINK".to_string()));
            assert!(rel.types.contains(&"REFERENCE".to_string()));
        }
    }
}

// Tests for aggregate functions (COLLECT, COUNT, etc.)

#[test]
fn test_parse_collect_aggregate() {
    // Test parsing COLLECT aggregate function
    let query = "MATCH (n:Person) RETURN collect(n.name) AS names";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse COLLECT query: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert_eq!(query.clauses.len(), 2); // MATCH and RETURN

    // Verify RETURN clause has COLLECT function
    if let raisin_cypher_parser::Clause::Return { items, .. } = &query.clauses[1] {
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].alias, Some("names".to_string()));

        if let raisin_cypher_parser::Expr::FunctionCall {
            name,
            args,
            distinct,
        } = &items[0].expr
        {
            assert_eq!(name.to_lowercase(), "collect");
            assert_eq!(args.len(), 1);
            assert_eq!(*distinct, false);
        } else {
            panic!("Expected FunctionCall expression");
        }
    } else {
        panic!("Second clause should be RETURN");
    }
}

// Note: COLLECT DISTINCT is not yet supported by the parser
// TODO: Add test when parser supports DISTINCT within function calls

#[test]
fn test_parse_collect_with_grouping() {
    // Test parsing COLLECT with grouping key
    let query =
        "MATCH (p:Person)-[:WORKS_AT]->(c:Company) RETURN c.name, collect(p.name) AS employees";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse COLLECT with grouping: {:?}",
        result.err()
    );

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Return { items, .. } = &query.clauses[1] {
        assert_eq!(items.len(), 2);

        // First item is non-aggregate (grouping key)
        if let raisin_cypher_parser::Expr::Property { .. } = &items[0].expr {
            // Good: c.name is property access
        } else {
            panic!("First item should be property access (grouping key)");
        }

        // Second item is aggregate (COLLECT)
        if let raisin_cypher_parser::Expr::FunctionCall { name, .. } = &items[1].expr {
            assert_eq!(name.to_lowercase(), "collect");
        } else {
            panic!("Second item should be COLLECT function");
        }
    }
}

#[test]
fn test_parse_multiple_aggregates() {
    // Test parsing multiple aggregate functions
    let query = "MATCH (n:Person) RETURN count(n) AS total, collect(n.name) AS names, avg(n.age) AS avg_age";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse multiple aggregates: {:?}",
        result.err()
    );

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Return { items, .. } = &query.clauses[1] {
        assert_eq!(items.len(), 3);

        // Verify each aggregate function
        let expected_funcs = vec!["count", "collect", "avg"];
        for (i, expected) in expected_funcs.iter().enumerate() {
            if let raisin_cypher_parser::Expr::FunctionCall { name, .. } = &items[i].expr {
                assert_eq!(
                    name.to_lowercase(),
                    *expected,
                    "Item {} should be {} function",
                    i,
                    expected
                );
            } else {
                panic!("Item {} should be function call", i);
            }
        }
    }
}

// Note: COUNT(*) syntax is not yet supported by the parser
// For now, use count(n) instead of count(*)
#[test]
fn test_parse_count_function() {
    // Test parsing COUNT function (without *)
    let query = "MATCH (n) RETURN count(n) AS total";
    let result = parse_query(query);
    assert!(result.is_ok(), "Should parse count(n): {:?}", result.err());

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Return { items, .. } = &query.clauses[1] {
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].alias, Some("total".to_string()));

        if let raisin_cypher_parser::Expr::FunctionCall { name, .. } = &items[0].expr {
            assert_eq!(name.to_lowercase(), "count");
        }
    }
}

#[test]
fn test_parse_sum_and_avg() {
    // Test parsing SUM and AVG aggregate functions
    let query = "MATCH (p:Product) RETURN sum(p.price) AS total, avg(p.rating) AS avg_rating";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse SUM and AVG: {:?}",
        result.err()
    );

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Return { items, .. } = &query.clauses[1] {
        assert_eq!(items.len(), 2);

        if let raisin_cypher_parser::Expr::FunctionCall { name, .. } = &items[0].expr {
            assert_eq!(name.to_lowercase(), "sum");
        }

        if let raisin_cypher_parser::Expr::FunctionCall { name, .. } = &items[1].expr {
            assert_eq!(name.to_lowercase(), "avg");
        }
    }
}

#[test]
fn test_parse_min_and_max() {
    // Test parsing MIN and MAX aggregate functions
    let query = "MATCH (p:Product) RETURN min(p.price) AS min_price, max(p.price) AS max_price";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse MIN and MAX: {:?}",
        result.err()
    );

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Return { items, .. } = &query.clauses[1] {
        assert_eq!(items.len(), 2);

        if let raisin_cypher_parser::Expr::FunctionCall { name, .. } = &items[0].expr {
            assert_eq!(name.to_lowercase(), "min");
        }

        if let raisin_cypher_parser::Expr::FunctionCall { name, .. } = &items[1].expr {
            assert_eq!(name.to_lowercase(), "max");
        }
    }
}

// Note: Full end-to-end testing with actual SQL engine execution
// and storage-backed relationship traversal is done in the server integration tests

#[test]
fn test_parse_variable_length_exact() {
    // Test parsing exact length pattern: (a)-[:KNOWS*2]->(b)
    let query = "MATCH (a)-[:KNOWS*2]->(b) RETURN a, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse exact variable-length pattern: {:?}",
        result.err()
    );

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];
        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert!(rel.range.is_some(), "Should have range for variable-length");
            let range = rel.range.as_ref().unwrap();
            assert_eq!(range.min, Some(2), "Should have min=2");
            assert_eq!(range.max, Some(2), "Should have max=2 for exact match");
        }
    }
}

#[test]
fn test_parse_variable_length_range() {
    // Test parsing range pattern: (a)-[:KNOWS*1..3]->(b)
    let query = "MATCH (a)-[:FRIEND*1..3]->(b) RETURN a, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse range variable-length pattern: {:?}",
        result.err()
    );

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];
        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert!(rel.range.is_some(), "Should have range");
            let range = rel.range.as_ref().unwrap();
            assert_eq!(range.min, Some(1), "Should have min=1");
            assert_eq!(range.max, Some(3), "Should have max=3");
            assert_eq!(
                rel.types,
                vec!["FRIEND".to_string()],
                "Should have FRIEND type"
            );
        }
    }
}

#[test]
fn test_parse_variable_length_unbounded() {
    // Test parsing unbounded pattern: (a)-[:LINK*]->(b)
    let query = "MATCH (a)-[:LINK*]->(b) RETURN a, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse unbounded variable-length pattern: {:?}",
        result.err()
    );

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];
        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert!(rel.range.is_some(), "Should have range");
            let range = rel.range.as_ref().unwrap();
            assert_eq!(range.min, None, "Should have no min (unbounded)");
            assert_eq!(range.max, None, "Should have no max (unbounded)");
        }
    }
}

#[test]
fn test_parse_variable_length_min_only() {
    // Test parsing min-only pattern: (a)-[:PATH*2..]->(b)
    let query = "MATCH (a)-[:PATH*2..]->(b) RETURN a, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse min-only variable-length pattern: {:?}",
        result.err()
    );

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];
        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert!(rel.range.is_some(), "Should have range");
            let range = rel.range.as_ref().unwrap();
            assert_eq!(range.min, Some(2), "Should have min=2");
            assert_eq!(range.max, None, "Should have no max");
        }
    }
}

#[test]
fn test_parse_variable_length_max_only() {
    // Test parsing max-only pattern: (a)-[:PATH*..5]->(b)
    let query = "MATCH (a)-[:PATH*..5]->(b) RETURN a, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse max-only variable-length pattern: {:?}",
        result.err()
    );

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];
        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert!(rel.range.is_some(), "Should have range");
            let range = rel.range.as_ref().unwrap();
            assert_eq!(range.min, None, "Should have no min");
            assert_eq!(range.max, Some(5), "Should have max=5");
        }
    }
}

#[test]
fn test_parse_variable_length_with_relationship_variable() {
    // Test parsing with relationship variable: (a)-[r:KNOWS*1..3]->(b)
    let query = "MATCH (a)-[r:KNOWS*1..3]->(b) RETURN a, r, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse variable-length with relationship variable: {:?}",
        result.err()
    );

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];
        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert_eq!(
                rel.variable,
                Some("r".to_string()),
                "Should have relationship variable 'r'"
            );
            assert!(rel.range.is_some(), "Should have range");
            let range = rel.range.as_ref().unwrap();
            assert_eq!(range.min, Some(1), "Should have min=1");
            assert_eq!(range.max, Some(3), "Should have max=3");
        }
    }

    // Verify RETURN clause includes the relationship variable
    if let raisin_cypher_parser::Clause::Return { items, .. } = &query.clauses[1] {
        assert_eq!(items.len(), 3, "Should return 3 items: a, r, b");
    }
}

#[test]
fn test_parse_variable_length_bidirectional() {
    // Test parsing bidirectional pattern: (a)-[:KNOWS*1..2]-(b)
    let query = "MATCH (a)-[:KNOWS*1..2]-(b) RETURN a, b";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse bidirectional variable-length pattern: {:?}",
        result.err()
    );

    let query = result.unwrap();
    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];
        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert!(
                matches!(
                    rel.direction,
                    raisin_cypher_parser::Direction::Both | raisin_cypher_parser::Direction::None
                ),
                "Should be bidirectional or undirected"
            );
            assert!(rel.range.is_some(), "Should have range");
        }
    }
}

#[test]
fn test_parse_friends_of_friends_pattern() {
    // Test the classic "friends of friends" query
    let query =
        "MATCH (me:Person)-[:FRIEND*2]->(fof:Person) WHERE me.id = 'user-123' RETURN fof.name";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse friends-of-friends pattern: {:?}",
        result.err()
    );

    let query = result.unwrap();
    assert_eq!(query.clauses.len(), 3); // MATCH, WHERE, RETURN

    // Verify MATCH clause has variable-length pattern
    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];
        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            assert!(rel.range.is_some(), "Should have range");
            let range = rel.range.as_ref().unwrap();
            assert_eq!(range.min, Some(2), "Should require exactly 2 hops");
            assert_eq!(range.max, Some(2), "Should require exactly 2 hops");
        }
    }
}

#[test]
fn test_parse_reachability_query() {
    // Test reachability query: find all nodes reachable within N hops
    let query = "MATCH (start)-[:LINK*1..5]->(reachable) WHERE start.id = 'node-1' RETURN DISTINCT reachable.id";
    let result = parse_query(query);
    assert!(
        result.is_ok(),
        "Should parse reachability pattern: {:?}",
        result.err()
    );

    let query = result.unwrap();

    // Verify variable-length pattern
    if let raisin_cypher_parser::Clause::Match { pattern, .. } = &query.clauses[0] {
        let path = &pattern.patterns[0];
        if let raisin_cypher_parser::PatternElement::Relationship(rel) = &path.elements[1] {
            let range = rel.range.as_ref().unwrap();
            assert_eq!(range.min, Some(1), "Should start from 1 hop");
            assert_eq!(range.max, Some(5), "Should go up to 5 hops");
        }
    }

    // Verify DISTINCT in RETURN
    if let raisin_cypher_parser::Clause::Return { distinct, .. } = &query.clauses[2] {
        assert!(*distinct, "Should have DISTINCT modifier");
    }
}
