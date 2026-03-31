# raisin-cypher-parser

Graph query pattern parser for RaisinDB, built with nom 8.0.

> **Deprecation Notice:** The standalone `CYPHER()` function is deprecated. For new graph queries, use SQL/PGQ `GRAPH_TABLE()` syntax instead. See [`raisin-sql`](../raisin-sql) for the recommended approach.
>
> This crate remains maintained for:
> - Legacy `CYPHER()` function support
> - Internal pattern parsing used by `GRAPH_TABLE()` implementation

## Overview

This crate provides parsing of graph pattern queries into a strongly-typed Abstract Syntax Tree (AST). It powers the graph query capabilities in RaisinDB by parsing the pattern matching syntax used in both standalone Cypher queries and the SQL/PGQ `GRAPH_TABLE()` function.

### When to Use What

| Syntax | Status | Use Case |
|--------|--------|----------|
| `GRAPH_TABLE(MATCH ... COLUMNS ...)` | **Recommended** | ISO SQL:2023 compliant, full SQL integration |
| `CYPHER('...')` | Deprecated | Legacy support only |

**For new projects, use `GRAPH_TABLE()`** - see [`raisin-sql/SUPPORTED_PGQ_FEATURES.md`](../raisin-sql/SUPPORTED_PGQ_FEATURES.md) for complete documentation.

- **Complete openCypher Parsing** - MATCH, CREATE, RETURN, WHERE, SET, DELETE, etc.
- **Pattern Matching** - Nodes, relationships, variable-length paths
- **Expression Parsing** - Full operator precedence, function calls, literals
- **Rich Error Messages** - Line and column position information
- **Type-Safe AST** - Strongly-typed with serde serialization support
- **Zero-Copy Parsing** - Built on nom for efficient parsing

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          SQL Query                                      │
│   SELECT * FROM GRAPH_TABLE(MATCH (a)-[:r]->(b) COLUMNS (...))         │
└───────────────────────────────────┬─────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                        raisin-sql-parser                                │
│                   Parses SQL syntax including GRAPH_TABLE               │
└───────────────────────────────────┬─────────────────────────────────────┘
                                    │ Pattern syntax extracted
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                     raisin-cypher-parser (this crate)                   │
│  ┌───────────────┐  ┌───────────────┐  ┌───────────────┐                │
│  │    Parser     │  │      AST      │  │    Errors     │                │
│  │  (nom 8.0)    │  │   (serde)     │  │  (positions)  │                │
│  └───────────────┘  └───────────────┘  └───────────────┘                │
│                                                                          │
│  Modules:                                                                │
│  • parser/clause.rs   - MATCH, CREATE, WHERE, RETURN parsing            │
│  • parser/pattern.rs  - Node and relationship patterns                  │
│  • parser/expr.rs     - Expressions with operator precedence            │
│  • parser/literal.rs  - NULL, booleans, numbers, strings                │
│  • ast/statement.rs   - Query, Clause, ReturnItem types                 │
│  • ast/pattern.rs     - GraphPattern, NodePattern, RelPattern           │
│  • ast/expr.rs        - Expr, Literal, BinOp, UnOp                      │
└───────────────────────────────────┬─────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                        raisin-sql-execution                             │
│                     Executes queries against storage                    │
└─────────────────────────────────────────────────────────────────────────┘
```

## Quick Start

```rust
use raisin_cypher_parser::{parse_query, parse_pattern, parse_expr};

// Parse a complete Cypher query
let query = parse_query(r#"
    MATCH (p:Person)-[:KNOWS]->(f:Person)
    WHERE p.age > 25
    RETURN p.name, f.name
    LIMIT 10
"#)?;

// Parse just a pattern
let pattern = parse_pattern("(a:User)-[:follows*1..3]->(b:User)")?;

// Parse just an expression
let expr = parse_expr("age > 18 AND active = true")?;
```

## API Reference

| Function | Description |
|----------|-------------|
| `parse_query(input)` | Parse complete Cypher query |
| `parse_statement(input)` | Parse statement (query/DDL) |
| `parse_pattern(input)` | Parse graph pattern |
| `parse_path(input)` | Parse single path pattern |
| `parse_expr(input)` | Parse individual expression |

### Extension Trait

```rust
use raisin_cypher_parser::CypherParse;

// Parse using the extension trait
let query = "MATCH (n:Person) RETURN n".parse_cypher()?;
```

## Supported Syntax

### Clauses

| Clause | Example |
|--------|---------|
| MATCH | `MATCH (n:Person)` |
| OPTIONAL MATCH | `OPTIONAL MATCH (n)-[:KNOWS]->(m)` |
| WHERE | `WHERE n.age > 25` |
| CREATE | `CREATE (n:Person {name: 'Alice'})` |
| MERGE | `MERGE (n:Person {id: 123})` |
| SET | `SET n.name = 'Bob'` |
| DELETE | `DELETE n` |
| REMOVE | `REMOVE n.age` |
| RETURN | `RETURN n.name AS name` |
| WITH | `WITH n.name AS name` |
| UNWIND | `UNWIND list AS item` |
| ORDER BY | `ORDER BY n.name DESC` |
| SKIP/LIMIT | `SKIP 10 LIMIT 5` |

### Patterns

```cypher
-- Nodes
(n)                          -- Any node
(n:Person)                   -- Labeled node
(n:Person:Employee)          -- Multiple labels
(n {name: 'Alice'})          -- With properties
(n:Person WHERE n.age > 25)  -- Inline filter (Cypher 10)

-- Relationships
-[r]->                       -- Outgoing
<-[r]-                       -- Incoming
-[r]-                        -- Any direction
-[:KNOWS]->                  -- Typed relationship
-[r:KNOWS|LIKES]->           -- Multiple types

-- Variable-length paths
-[*]->                       -- Any length (1..10 default)
-[*2]->                      -- Exactly 2 hops
-[*1..3]->                   -- 1 to 3 hops
-[*..5]->                    -- Up to 5 hops
-[*2..]->                    -- 2 or more hops
```

### Expressions

Operator precedence (lowest to highest):

1. `OR`
2. `XOR`
3. `AND`
4. `NOT`
5. Comparison: `=`, `<>`, `<`, `<=`, `>`, `>=`
6. String: `STARTS WITH`, `ENDS WITH`, `CONTAINS`, `=~`
7. `IN`
8. Arithmetic: `+`, `-`
9. Arithmetic: `*`, `/`, `%`
10. Power: `^`
11. Unary: `+`, `-`, `IS NULL`, `IS NOT NULL`
12. Property access: `.`
13. Atoms: literals, variables, function calls

## Modules

| Module | Types |
|--------|-------|
| `ast::statement` | `Statement`, `Query`, `Clause`, `ReturnItem`, `OrderBy`, `SetItem`, `RemoveItem` |
| `ast::pattern` | `GraphPattern`, `PathPattern`, `NodePattern`, `RelPattern`, `Direction`, `Range` |
| `ast::expr` | `Expr`, `Literal`, `BinOp`, `UnOp` |

## Error Handling

```rust
use raisin_cypher_parser::{parse_query, ParseError};

match parse_query("MATCH (n RETURN n") {
    Ok(query) => println!("Parsed: {:?}", query),
    Err(ParseError::SyntaxError { line, column, message }) => {
        eprintln!("Syntax error at {}:{}: {}", line, column, message);
    }
    Err(e) => eprintln!("Error: {}", e),
}
```

Error types:
- `ParseError::SyntaxError` - Syntax error with position
- `ParseError::UnexpectedToken` - Expected vs found mismatch
- `ParseError::InvalidSyntax` - Invalid construct
- `ParseError::UnexpectedEof` - Unexpected end of input
- `ParseError::Incomplete` - Input not fully consumed

## Contributing

This is an open source project and contributions are welcome!

### Getting Started

1. Clone the repository
2. Run tests: `cargo test -p raisin-cypher-parser`
3. Check formatting: `cargo fmt --check`
4. Run clippy: `cargo clippy`

### Areas for Contribution

- **Parser improvements** - Additional Cypher syntax support
- **Error messages** - Better error recovery and suggestions
- **Performance** - Benchmarks and optimizations
- **Documentation** - Examples and tutorials

## Related Crates

- [`raisin-sql`](../raisin-sql) - SQL/PGQ parser and AST (recommended for graph queries)
- [`raisin-sql-execution`](../raisin-sql-execution) - Query execution engine
- [`raisin-rocksdb`](../raisin-rocksdb) - Graph storage layer

## References

- [openCypher Grammar (BNF)](https://github.com/opencypher/openCypher/blob/main/grammar/openCypher.bnf)
- [ISO SQL:2023 Part 16 (SQL/PGQ)](https://www.iso.org/standard/76583.html)
- [nom 8.0 Documentation](https://docs.rs/nom/latest/nom/)

## License

[BSL-1.1](../../LICENSE)
