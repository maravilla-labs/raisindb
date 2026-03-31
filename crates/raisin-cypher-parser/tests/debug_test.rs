// SPDX-License-Identifier: BSL-1.1

use raisin_cypher_parser::*;

#[test]
fn debug_where_clause() {
    let query = "MATCH (a)-[:KNOWS]->(b) WHERE a.name = 'Alice' RETURN b.name";
    let result = parse_query(query).unwrap();

    println!("Number of clauses: {}", result.clauses.len());
    for (i, clause) in result.clauses.iter().enumerate() {
        println!("Clause {}: {:?}", i, clause);
    }
}

#[test]
fn debug_complex_query() {
    let query = r#"
        MATCH (a:Person), (b:Person)
        WHERE a.name = 'Node A' AND b.name = 'Node B'
        CREATE (a)-[e:RELTYPE {name: a.name + '<->' + b.name}]->(b)
        RETURN e
    "#;

    let result = parse_query(query).unwrap();

    println!("Number of clauses: {}", result.clauses.len());
    for (i, clause) in result.clauses.iter().enumerate() {
        match clause {
            Clause::Match { .. } => println!("Clause {}: MATCH", i),
            Clause::Where { .. } => println!("Clause {}: WHERE", i),
            Clause::Create { .. } => println!("Clause {}: CREATE", i),
            Clause::Return { .. } => println!("Clause {}: RETURN", i),
            _ => println!("Clause {}: Other", i),
        }
    }
}
