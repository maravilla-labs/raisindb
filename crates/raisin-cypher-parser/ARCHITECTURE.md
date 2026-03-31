# raisin-cypher-parser Architecture

This document describes the internal architecture and design decisions of the graph query parser.

## Design Philosophy

The parser follows several key principles:

1. **Separation of Concerns** - AST types are separate from parsing logic
2. **Composability** - Individual parsers combine to parse complex structures
3. **Zero-Copy Where Possible** - Uses nom's zero-copy parsing with `&str` slices
4. **Rich Error Context** - Position tracking for all parse errors
5. **Serde-First** - All AST types are serializable for tooling integration

## Module Organization

```
src/
├── lib.rs              Public API entry points
├── error.rs            ParseError type with position info
├── parser.rs           Internal parser module exports
├── ast/                Abstract Syntax Tree types
│   ├── mod.rs          Module exports + Span type
│   ├── statement.rs    Query, Clause, ReturnItem
│   ├── pattern.rs      GraphPattern, NodePattern, RelPattern
│   └── expr.rs         Expr, Literal, BinOp, UnOp
└── parser/             Parser combinators (nom)
    ├── common.rs       Utilities: whitespace, identifiers
    ├── literal.rs      Literal values: null, bool, number, string
    ├── expr.rs         Expression parsing with precedence
    ├── pattern.rs      Pattern parsing: nodes, relationships
    └── clause.rs       Clause parsing: MATCH, WHERE, RETURN
```

## Parser Pipeline

```
                    Input String
                         │
                         ▼
┌─────────────────────────────────────────────────────────┐
│                   nom_locate::LocatedSpan              │
│              Wraps input with position tracking         │
└─────────────────────────────┬───────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────┐
│                    parser/clause.rs                     │
│  query() → statement() → clause() → specific clauses    │
└─────────────────────────────┬───────────────────────────┘
                              │
           ┌──────────────────┼──────────────────┐
           │                  │                  │
           ▼                  ▼                  ▼
    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
    │   pattern   │    │    expr     │    │   literal   │
    │  parsing    │    │  parsing    │    │  parsing    │
    └─────────────┘    └─────────────┘    └─────────────┘
           │                  │                  │
           └──────────────────┼──────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────┐
│                        AST Types                        │
│     Query { clauses: Vec<Clause> }                     │
│     Clause::Match { pattern, optional }                │
│     Clause::Where { expr }                             │
│     Clause::Return { items, distinct, order, ... }     │
└─────────────────────────────────────────────────────────┘
```

## Core Types

### Position Tracking

```rust
// From nom_locate - wraps &str with position info
pub type Span<'a> = LocatedSpan<&'a str>;

// Our position type for AST nodes
#[derive(Debug, Clone, Serialize)]
pub struct Span {
    pub start: usize,   // Byte offset from start
    pub end: usize,     // End byte offset
    pub line: u32,      // 1-indexed line number
    pub column: u32,    // 1-indexed column
}
```

### AST Hierarchy

```
Statement
    └── Query { clauses: Vec<Clause> }
            │
            ├── Clause::Match { pattern: GraphPattern, optional: bool }
            │       └── GraphPattern { paths: Vec<PathPattern>, where_: Option<Expr> }
            │               └── PathPattern { name: Option<String>, elements: Vec<PatternElement> }
            │                       ├── PatternElement::Node(NodePattern)
            │                       │       ├── variable: Option<String>
            │                       │       ├── labels: Vec<String>
            │                       │       ├── properties: Option<Expr>
            │                       │       └── where_: Option<Expr>
            │                       └── PatternElement::Relationship(RelPattern)
            │                               ├── variable: Option<String>
            │                               ├── types: Vec<String>
            │                               ├── properties: Option<Expr>
            │                               ├── direction: Direction
            │                               └── range: Option<Range>
            │
            ├── Clause::Where { expr: Expr }
            │
            ├── Clause::Return { items, distinct, order_by, skip, limit }
            │       └── ReturnItem { expr: Expr, alias: Option<String> }
            │
            ├── Clause::Create { pattern: GraphPattern }
            ├── Clause::Merge { pattern: PathPattern, on_create, on_match }
            ├── Clause::Set { items: Vec<SetItem> }
            ├── Clause::Delete { exprs, detach }
            ├── Clause::Remove { items: Vec<RemoveItem> }
            ├── Clause::With { items, distinct, order_by, skip, limit, where_ }
            └── Clause::Unwind { expr, alias }
```

### Expression Types

```
Expr
    ├── Literal(Literal)
    │       ├── Null
    │       ├── Boolean(bool)
    │       ├── Integer(i64)
    │       ├── Float(f64)
    │       └── String(String)
    │
    ├── Variable(String)
    ├── Parameter(String)           // $param
    ├── Property(Box<Expr>, String) // expr.property
    │
    ├── BinaryOp { left, op, right }
    │       └── BinOp: Or, Xor, And, Eq, Ne, Lt, Le, Gt, Ge,
    │                  StartsWith, EndsWith, Contains, RegexMatch,
    │                  Add, Sub, Mul, Div, Mod, Pow, In
    │
    ├── UnaryOp { op, expr }
    │       └── UnOp: Not, Plus, Minus, IsNull, IsNotNull
    │
    ├── FunctionCall { name, args }
    ├── List(Vec<Expr>)
    ├── Map(Vec<(String, Expr)>)
    └── Case { expr, when_clauses, else_clause }
```

## Parser Combinator Design

### Using nom 8.0

The parser uses nom's combinator approach:

```rust
// Type alias for parser results
pub type PResult<'a, O> = IResult<Span<'a>, O, nom::error::Error<Span<'a>>>;

// Example: parsing a keyword (case-insensitive)
pub fn keyword<'a>(kw: &'static str) -> impl Fn(Span<'a>) -> PResult<'a, Span<'a>> {
    move |input| tag_no_case(kw)(input)
}

// Example: parsing whitespace including comments
pub fn ws0(input: Span) -> PResult<Span> {
    recognize(many0(alt((multispace1, line_comment, block_comment))))(input)
}
```

### Expression Precedence

Expressions use precedence climbing:

```rust
// Lowest precedence: OR
fn or_expr(input: Span) -> PResult<Expr> {
    let (input, first) = xor_expr(input)?;
    fold_many0(
        preceded(ws_keyword("OR"), xor_expr),
        move || first.clone(),
        |acc, val| Expr::BinaryOp {
            left: Box::new(acc),
            op: BinOp::Or,
            right: Box::new(val),
        },
    )(input)
}

// Next level: XOR (calls OR for lower precedence)
fn xor_expr(input: Span) -> PResult<Expr> {
    // ... calls and_expr
}

// Continue up the precedence chain...
```

Precedence levels (lowest to highest):

| Level | Operators |
|-------|-----------|
| 1 | OR |
| 2 | XOR |
| 3 | AND |
| 4 | NOT |
| 5 | = <> < <= > >= |
| 6 | STARTS WITH, ENDS WITH, CONTAINS, =~ |
| 7 | IN |
| 8 | + - |
| 9 | * / % |
| 10 | ^ |
| 11 | unary +, -, IS NULL, IS NOT NULL |
| 12 | property access (.) |
| 13 | atoms (literals, variables, parens) |

### Pattern Parsing

Graph patterns are parsed as alternating node and relationship elements:

```rust
// (a)-[r]->(b)-[s]->(c)
// Becomes: Node, Rel, Node, Rel, Node

fn path_elements(input: Span) -> PResult<Vec<PatternElement>> {
    let (input, first) = node_pattern(input)?;
    let (input, rest) = many0(pair(rel_pattern, node_pattern))(input)?;

    let mut elements = vec![PatternElement::Node(first)];
    for (rel, node) in rest {
        elements.push(PatternElement::Relationship(rel));
        elements.push(PatternElement::Node(node));
    }
    Ok((input, elements))
}
```

### Relationship Direction Parsing

Direction is determined by arrow syntax:

```rust
// -[...]->  : Direction::Right (outgoing)
// <-[...]-  : Direction::Left (incoming)
// -[...]-   : Direction::None (any direction)
// <-[...]-> : Direction::Both (bidirectional)

fn rel_pattern(input: Span) -> PResult<RelPattern> {
    let (input, left_arrow) = opt(char('<'))(input)?;
    let (input, _) = char('-')(input)?;
    let (input, inner) = opt(rel_inner)(input)?;
    let (input, _) = char('-')(input)?;
    let (input, right_arrow) = opt(char('>'))(input)?;

    let direction = match (left_arrow.is_some(), right_arrow.is_some()) {
        (false, true) => Direction::Right,
        (true, false) => Direction::Left,
        (true, true) => Direction::Both,
        (false, false) => Direction::None,
    };
    // ...
}
```

## Error Handling

### Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("Syntax error at line {line}, column {column}: {message}")]
    SyntaxError {
        line: u32,
        column: u32,
        message: String,
    },

    #[error("Unexpected token at line {line}, column {column}: expected {expected}, found {found}")]
    UnexpectedToken {
        line: u32,
        column: u32,
        expected: String,
        found: String,
    },

    #[error("Invalid syntax: {0}")]
    InvalidSyntax(String),

    #[error("Unexpected end of input")]
    UnexpectedEof,

    #[error("Incomplete parse: {0}")]
    Incomplete(String),
}
```

### Error Recovery

The parser converts nom errors to our error type with position info:

```rust
fn convert_error(input: Span, err: nom::error::Error<Span>) -> ParseError {
    let pos = err.input;
    ParseError::SyntaxError {
        line: pos.location_line(),
        column: pos.get_utf8_column() as u32,
        message: format!("near '{}'", &pos.fragment()[..20.min(pos.len())]),
    }
}
```

## Testing Strategy

### Unit Tests

Each parser module has inline tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_pattern_simple() {
        let result = parse_pattern("(n)").unwrap();
        assert!(matches!(result.paths[0].elements[0],
            PatternElement::Node(NodePattern { variable: Some(ref v), .. }) if v == "n"));
    }
}
```

### Integration Tests

`tests/integration_tests.rs` contains end-to-end tests:

```rust
#[test]
fn test_complex_query() {
    let input = r#"
        MATCH (p:Person)-[:KNOWS]->(f:Person)
        WHERE p.age > 25 AND f.city = 'NYC'
        WITH p, collect(f) AS friends
        RETURN p.name, size(friends) AS friend_count
        ORDER BY friend_count DESC
        LIMIT 10
    "#;

    let query = parse_query(input).expect("Failed to parse");
    assert_eq!(query.clauses.len(), 5);
}
```

## Future Considerations

### Planned Improvements

1. **Better Error Recovery** - Continue parsing after errors to report multiple issues
2. **Streaming Support** - Parse very large queries without loading entirely in memory
3. **Validation Pass** - Semantic validation after parsing (undefined variables, etc.)
4. **Query Normalization** - Canonicalize AST for comparison and caching

### Performance Notes

- nom's zero-copy parsing minimizes allocations
- String interning could reduce memory for repeated identifiers
- AST pooling could help for high-throughput parsing scenarios
