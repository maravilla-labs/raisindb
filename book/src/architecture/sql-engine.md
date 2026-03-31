# SQL Engine and PostgreSQL Wire Protocol

RaisinDB includes a full SQL engine that lets you query content using standard SQL syntax, extended with hierarchical path operations, JSON queries, vector search, and graph traversal. You can connect using any PostgreSQL client via the built-in pgwire transport.

## Architecture

The SQL pipeline is split across three crates:

```
SQL String
    │
    ▼
┌────────────────────────────────┐
│  raisin-sql (WASM-compatible)  │
│  ┌──────────┐                  │
│  │  Parser   │ ─► AST          │
│  └──────────┘                  │
│  ┌──────────┐                  │
│  │ Analyzer  │ ─► Typed AST    │
│  └──────────┘                  │
│  ┌──────────┐                  │
│  │ Planner   │ ─► Logical Plan │
│  └──────────┘                  │
│  ┌──────────┐                  │
│  │ Optimizer │ ─► Optimized    │
│  └──────────┘    Logical Plan  │
└────────────────────────────────┘
    │
    ▼
┌────────────────────────────────┐
│     raisin-sql-execution       │
│  ┌────────────────┐            │
│  │Physical Planner│ ─► Scans   │
│  └────────────────┘            │
│  ┌────────────────┐            │
│  │   Executor     │ ─► Rows    │
│  └────────────────┘            │
│  ┌────────────────┐            │
│  │  QueryEngine   │ High-level │
│  └────────────────┘    API     │
└────────────────────────────────┘
    │
    ▼
┌────────────────────────────────┐
│   raisin-transport-pgwire      │
│  PostgreSQL wire protocol      │
│  (psql, pgAdmin, any PG       │
│   client library)              │
└────────────────────────────────┘
```

The `raisin-sql` crate is deliberately WASM-compatible -- it contains only parsing, analysis, and logical planning with no runtime dependencies. The `raisin-sql-execution` crate adds the physical execution layer with RocksDB, tokio, and storage integration.

## Query Pipeline

### 1. Parsing

The parser converts SQL strings into an Abstract Syntax Tree using the `sqlparser` crate with a custom `RaisinDialect`:

```rust
use raisin_sql::{parse_sql, RaisinDialect};

let sql = "SELECT id, name FROM nodes WHERE PATH_STARTS_WITH(path, '/content/') LIMIT 10";
let statements = parse_sql(sql).unwrap();
```

### 2. Semantic Analysis

The analyzer performs type checking and semantic validation. It uses a `Catalog` trait to resolve table schemas (workspaces map to tables):

```rust
use raisin_sql::{Analyzer, StaticCatalog};

let catalog = StaticCatalog::default_nodes_schema();
let analyzer = Analyzer::with_catalog(Box::new(catalog));
let analyzed = analyzer.analyze(sql).unwrap();
```

### 3. Logical Planning

The plan builder transforms analyzed queries into a tree of logical operators (Scan, Filter, Project, Sort, Limit, Join, Aggregate):

```rust
use raisin_sql::PlanBuilder;

let planner = PlanBuilder::new(&catalog);
let plan = planner.build(&analyzed).unwrap();
println!("{}", plan.explain());
```

### 4. Optimization

The optimizer applies several passes:

- **Constant Folding** -- evaluates deterministic expressions at plan time (`DEPTH('/content/')` becomes `1`)
- **Hierarchy Rewriting** -- transforms path functions into efficient prefix scans (`PATH_STARTS_WITH(path, '/x/')` becomes a RocksDB prefix range scan)
- **Common Subexpression Elimination** -- extracts repeated expressions to avoid redundant computation
- **Projection Pruning** -- computes the minimal column set needed and pushes it down to the scan

```rust
use raisin_sql::{Optimizer, OptimizerConfig};

let optimizer = Optimizer::new();
let optimized = optimizer.optimize(plan);
```

### 5. Physical Execution

The physical planner selects concrete execution strategies. The `QueryEngine` provides a high-level streaming API:

```rust
use raisin_sql_execution::QueryEngine;
use futures::StreamExt;

let engine = QueryEngine::new(storage, "tenant1", "repo1", "main");
let mut stream = engine.execute("SELECT * FROM default WHERE __revision = 100").await?;

while let Some(row) = stream.next().await {
    println!("{:?}", row?);
}
```

Physical scan types include Table scans, Prefix scans (for hierarchical queries), Property index lookups, and Full-text search scans.

## RaisinDB SQL Extensions

### Workspace-as-Table

In RaisinDB, each workspace maps to a SQL table. The `default` workspace is queried as:

```sql
SELECT * FROM 'default' WHERE node_type = 'article'
```

### Hierarchical Path Functions

```sql
-- Find all nodes under /content/
SELECT * FROM 'default' WHERE PATH_STARTS_WITH(path, '/content/')

-- Find direct children of /content/
SELECT * FROM 'default' WHERE PARENT(path) = '/content'

-- Filter by depth in the tree
SELECT * FROM 'default' WHERE DEPTH(path) = 2
```

### JSON Property Queries

Access node properties stored as JSON using the `->>` operator. Cast the key to `String`:

```sql
-- Correct: Cast the key
SELECT * FROM 'default' WHERE properties->>'email'::String = 'user@example.com'

-- Wrong: Cast the result (causes type coercion error)
SELECT * FROM 'default' WHERE (properties->>'email')::String = 'user@example.com'
```

### Parameter Substitution

Prepared statements use positional parameters (`$1`, `$2`, ...):

```sql
SELECT * FROM 'default' WHERE properties->>'user_id'::String = $1 AND node_type = $2
```

## PostgreSQL Wire Protocol (pgwire)

The `raisin-transport-pgwire` crate implements the PostgreSQL wire protocol, allowing any PostgreSQL client to connect to RaisinDB.

### Connecting

Start the server with pgwire enabled:

```bash
RUST_LOG=info ./target/release/raisin-server \
    --config node.toml \
    --pgwire-enabled true
```

Connect with `psql`:

```bash
psql -h 127.0.0.1 -p 5432 -U tenant1/repo1/main
```

The username encodes the tenant, repository, and branch context.

### Configuration

```rust
use raisin_transport_pgwire::{PgWireConfig, PgWireServer};

let config = PgWireConfig::builder()
    .bind_addr("0.0.0.0:5432")
    .max_connections(100)
    .build();
```

### Protocol Support

The pgwire transport supports:

- **Simple Query Protocol** -- text-based queries sent as plain SQL strings
- **Extended Query Protocol** -- prepared statements with binary parameter encoding
- **Authentication** -- password-based and no-auth modes, with API key validation
- **Type Mapping** -- automatic conversion between PostgreSQL types and RaisinDB property types (text, integer, float, boolean, JSON, timestamps, UUIDs)

### Supported Features

| Feature | Status |
|---------|--------|
| SELECT queries | Full support |
| INSERT/UPDATE/DELETE (DML) | Full support |
| Prepared statements | Full support |
| Binary encoding | Full support |
| Transaction blocks | Supported |
| PostgreSQL system catalogs | `pg_catalog` queries for client compatibility |

## Cypher and Graph Queries

The SQL engine integrates with the Cypher parser (`raisin-cypher-parser`) for graph pattern matching. You can execute Cypher queries through SQL using the `cypher()` function or the SQL/PGQ `GRAPH_TABLE` syntax:

```sql
-- Execute a Cypher query through SQL
SELECT * FROM cypher('MATCH (n:Person)-[:KNOWS]->(m:Person) RETURN n.name, m.name')

-- SQL/PGQ graph pattern matching
SELECT * FROM GRAPH_TABLE (social_graph
    MATCH (p:Person)-[k:KNOWS]->(f:Person)
    COLUMNS (p.name AS person, f.name AS friend)
)
```

See the [Graph Queries and Algorithms](../guides/graph-queries.md) chapter for more details.
