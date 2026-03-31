# raisin-query

Lightweight in-memory query engine for filtering, sorting, and paginating nodes in RaisinDB.

## Overview

This crate provides a declarative query language for searching node collections with:

- **Logical Operators**: AND, OR, NOT for composable filter expressions
- **Field Operators**: Equality, pattern matching, range comparisons, existence checks
- **Sorting**: Multi-field ordering with ascending/descending support
- **Pagination**: Offset-based pagination with limit/offset

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Query Pipeline                          │
│                                                              │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────┐   │
│  │    AST      │     │   Filter    │     │   Sort &    │   │
│  │  (Query)    │ ──> │   Engine    │ ──> │  Paginate   │   │
│  └─────────────┘     └─────────────┘     └─────────────┘   │
│                                                              │
│  NodeSearchQuery      filter_nodes()      eval_query()       │
│  Filter, FieldOps     matches_filter()    order_by, limit    │
└─────────────────────────────────────────────────────────────┘
```

## Query Structure

### NodeSearchQuery

Top-level query object combining filters, sorting, and pagination:

```rust
NodeSearchQuery {
    and: Option<Vec<Filter>>,              // All must match
    or: Option<Vec<Filter>>,               // Any must match
    not: Option<Box<Filter>>,              // Must not match
    order_by: Option<HashMap<String, SortOrder>>,
    limit: Option<usize>,
    offset: Option<usize>,
}
```

### Filter Types

| Type | Description | Example |
|------|-------------|---------|
| `Filter::And` | All sub-filters must match | `{ "and": [...] }` |
| `Filter::Or` | Any sub-filter must match | `{ "or": [...] }` |
| `Filter::Not` | Inverts sub-filter | `{ "not": {...} }` |
| `Filter::Field` | Field-level predicate | `{ "name": { "eq": "..." } }` |

### Field Operators

| Operator | Description | Value Type |
|----------|-------------|------------|
| `eq` | Equals | `Value` |
| `ne` | Not equals | `Value` |
| `like` | Contains substring | `String` |
| `contains` | String contains | `String` |
| `in` | Value in set | `Vec<Value>` |
| `exists` | Field existence | `bool` |
| `gt` / `gte` | Greater than (or equal) | `Value` |
| `lt` / `lte` | Less than (or equal) | `Value` |

### Supported Fields

Currently supports top-level node fields:

| Field | Aliases | Description |
|-------|---------|-------------|
| `id` | - | Unique node identifier |
| `name` | - | Display name |
| `path` | - | Hierarchical path |
| `node_type` | `nodeType` | Entity type |
| `parent` | - | Parent node reference |

## Usage

### Basic Filtering

```rust
use raisin_query::{NodeSearchQuery, Filter, FieldFilter, FieldOperators};
use std::collections::HashMap;

let query = NodeSearchQuery {
    and: Some(vec![
        Filter::Field(FieldFilter(HashMap::from([(
            "node_type".into(),
            FieldOperators {
                eq: Some(serde_json::Value::String("User".into())),
                ..Default::default()
            },
        )]))),
    ]),
    ..Default::default()
};

let results = raisin_query::eval_query(&nodes, &query);
```

### Pattern Matching

```rust
// Find nodes with paths containing "/users/"
let query = NodeSearchQuery {
    and: Some(vec![
        Filter::Field(FieldFilter(HashMap::from([(
            "path".into(),
            FieldOperators {
                like: Some("/users/".into()),
                ..Default::default()
            },
        )]))),
    ]),
    ..Default::default()
};
```

### Set Membership

```rust
use serde_json::Value;

// Find nodes of type "User" or "Admin"
let query = NodeSearchQuery {
    and: Some(vec![
        Filter::Field(FieldFilter(HashMap::from([(
            "nodeType".into(),
            FieldOperators {
                in_: Some(vec![
                    Value::String("User".into()),
                    Value::String("Admin".into()),
                ]),
                ..Default::default()
            },
        )]))),
    ]),
    ..Default::default()
};
```

### Sorting and Pagination

```rust
let query = NodeSearchQuery {
    order_by: Some(HashMap::from([
        ("path".into(), SortOrder::Asc),
    ])),
    limit: Some(20),
    offset: Some(40),  // Page 3
    ..Default::default()
};

let page = raisin_query::eval_query(&nodes, &query);
```

### JSON Serialization

Queries are fully serializable for API transport:

```json
{
  "and": [
    { "path": { "like": "/users/" } },
    { "nodeType": { "in": ["User", "Admin"] } }
  ],
  "not": { "name": { "eq": "guest" } },
  "order_by": { "path": "asc" },
  "limit": 20,
  "offset": 0
}
```

## API Reference

### Functions

| Function | Description |
|----------|-------------|
| `eval_query(&nodes, &query)` | Full pipeline: filter → sort → paginate |
| `filter_nodes(&nodes, &query)` | Filtering only, no sort/pagination |

## Modules

| Module | Description |
|--------|-------------|
| `ast.rs` | Query AST types (NodeSearchQuery, Filter, FieldOperators) |
| `executor.rs` | Query evaluation engine |

## Performance

- **Filtering**: O(n) where n = node count
- **Sorting**: O(n log n) when `order_by` specified
- **Memory**: References only, no cloning of nodes

## Integration

Used by higher-level crates for node search operations:

```
┌─────────────────┐     ┌─────────────────┐
│ raisin-core     │     │ raisin-server   │
│ (Node Service)  │ ──> │ (Search API)    │
└────────┬────────┘     └─────────────────┘
         │
         ▼
┌─────────────────┐
│  raisin-query   │
│  (This crate)   │
└─────────────────┘
         │
         ▼
┌─────────────────┐
│  raisin-models  │
│  (Node type)    │
└─────────────────┘
```

## Dependencies

```toml
[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
raisin-models = { path = "../raisin-models" }
```

## Future Enhancements

- Property-level filtering (`properties.email`, `properties.status`)
- Full-text search integration
- Query parser for string-based DSL

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
