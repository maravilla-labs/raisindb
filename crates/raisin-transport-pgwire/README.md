# raisin-transport-pgwire

PostgreSQL wire protocol transport layer for RaisinDB, enabling standard PostgreSQL clients to connect and query.

## Overview

This crate implements the PostgreSQL wire protocol (pgwire) for RaisinDB, allowing any PostgreSQL-compatible client (psql, JDBC, pgAdmin, DBeaver, etc.) to connect and execute SQL queries.

- **Full Protocol Support** - Simple query (text) and extended query (prepared statements) protocols
- **API Key Authentication** - Secure authentication using RaisinDB API keys
- **Type Mapping** - Bidirectional conversion between RaisinDB and PostgreSQL types
- **Binary Protocol** - Efficient binary encoding for JDBC drivers
- **Session Management** - Branch selection, identity context (RLS)
- **Connection Pooling** - Configurable connection limits with semaphore-based management

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                      PostgreSQL Clients                              │
│  (psql, JDBC, pgAdmin, DBeaver, Datagrip, Python psycopg2, etc.)    │
└────────────────────────────────┬────────────────────────────────────┘
                                 │ PostgreSQL Wire Protocol (port 5432)
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         PgWireServer                                 │
│  ┌─────────────┐  ┌─────────────┐  ┌────────────────┐              │
│  │   TCP       │  │  Connection │  │  Handler       │              │
│  │  Listener   │──│  Semaphore  │──│  Factory       │              │
│  └─────────────┘  └─────────────┘  └────────────────┘              │
└────────────────────────────────┬────────────────────────────────────┘
                                 │
          ┌──────────────────────┼──────────────────────┐
          ▼                      ▼                      ▼
┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
│ RaisinAuthHandler│  │ SimpleQuery      │  │ ExtendedQuery    │
│                  │  │ Handler          │  │ Handler          │
│ - API key auth   │  │                  │  │                  │
│ - Tenant/repo    │  │ - Text queries   │  │ - Parse/Bind     │
│ - Session ctx    │  │ - Multi-stmt     │  │ - Describe       │
│ - Identity (JWT) │  │ - System cmds    │  │ - Execute        │
└────────┬─────────┘  └────────┬─────────┘  └────────┬─────────┘
         │                     │                     │
         └─────────────────────┼─────────────────────┘
                               ▼
                    ┌──────────────────────┐
                    │     QueryEngine      │
                    │   (raisin-sql-exec)  │
                    └──────────────────────┘
                               │
                    ┌──────────┴──────────┐
                    ▼                     ▼
          ┌──────────────────┐  ┌──────────────────┐
          │  ResultEncoder   │  │  Type Mapping    │
          │  - Text format   │  │  - to_pg_type()  │
          │  - Binary format │  │  - Text encode   │
          │  - Schema infer  │  │  - Binary encode │
          └──────────────────┘  └──────────────────┘
```

## Connection String

```
postgresql://{tenant_id}:{api_key}@{host}:5432/{repository}
```

| Component | Description |
|-----------|-------------|
| `tenant_id` | Your RaisinDB tenant ID (username field) |
| `api_key` | API key with pgwire_access permission |
| `host` | RaisinDB server hostname |
| `5432` | PostgreSQL default port (configurable) |
| `repository` | Target repository name |

## Usage

### Server Setup

```rust
use raisin_transport_pgwire::{PgWireConfig, PgWireServer};

// Configure server
let config = PgWireConfig::builder()
    .bind_addr("0.0.0.0:5432")
    .max_connections(100)
    .build();

// Create server with handler factory
let server = PgWireServer::new(config)
    .with_handler(my_handler_factory);

// Run server (blocking)
server.run().await?;
```

### Client Connection (psql)

```bash
psql "postgresql://tenant123:raisin_api_xxx@localhost:5432/myrepo"
```

### JDBC Connection

```java
String url = "jdbc:postgresql://localhost:5432/myrepo";
Properties props = new Properties();
props.setProperty("user", "tenant123");
props.setProperty("password", "raisin_api_xxx");
Connection conn = DriverManager.getConnection(url, props);
```

## Session Commands

### Branch Selection

```sql
-- Set session branch
SET app.branch = 'feature-branch';
USE BRANCH 'feature-branch';

-- Show current branch
SHOW app.branch;
SHOW CURRENT BRANCH;
```

### Identity Context (Row-Level Security)

```sql
-- Set identity from JWT (for RLS)
SET app.user = 'eyJhbGciOiJIUzI1NiIs...';

-- Clear identity context
RESET app.user;
```

### System Queries

```sql
SELECT version();           -- RaisinDB version info
SHOW server_version;        -- PostgreSQL compatibility version
SHOW transaction_isolation; -- Transaction isolation level
```

## Modules

| Module | Description |
|--------|-------------|
| `server` | TCP server, connection management, config builder |
| `auth` | API key authentication, connection context |
| `simple_query` | Text-based query protocol (psql default) |
| `extended_query` | Prepared statements, parameter binding |
| `result_encoder` | Row encoding, schema inference, streaming |
| `type_mapping` | PropertyValue to PostgreSQL type conversion |
| `type_mapping_binary` | Binary protocol encoding (JDBC) |
| `error` | Error types and PostgreSQL error code mapping |

## Type Mappings

| RaisinDB Type | PostgreSQL Type | Notes |
|---------------|-----------------|-------|
| `Boolean` | `BOOL` | |
| `Integer` | `INT8` | 64-bit |
| `Float` | `FLOAT8` | Double precision |
| `Decimal` | `NUMERIC` | Arbitrary precision |
| `String` | `TEXT` | UTF-8 |
| `Date` | `TIMESTAMPTZ` | With timezone |
| `Vector` | `FLOAT4[]` | pgvector compatible |
| `Reference` | `JSONB` | Serialized |
| `Url` | `JSONB` | Serialized |
| `Object` | `JSONB` | Key-value maps |
| `Array` | `JSONB` | Heterogeneous |
| `Geometry` | `JSONB` | GeoJSON |

## Features

```toml
[dependencies]
raisin-transport-pgwire = { version = "0.1", features = ["indexing"] }
```

| Feature | Description |
|---------|-------------|
| `default` | Core pgwire support with indexing |
| `indexing` | Full-text search (Tantivy) and vector search (HNSW) |

## Protocol Support

| Protocol | Status | Description |
|----------|--------|-------------|
| Simple Query | Full | Text queries, multi-statement |
| Extended Query | Full | Parse, Bind, Describe, Execute |
| COPY | Stub | Not implemented |
| SSL/TLS | Planned | Connection encryption |

## Error Codes

Errors map to standard PostgreSQL SQLSTATE codes:

| Error Type | Code | Description |
|------------|------|-------------|
| Authentication | `28000` | Invalid authorization |
| Invalid Password | `28P01` | Bad API key |
| Syntax Error | `42601` | SQL parse error |
| Type Mismatch | `42804` | Datatype conversion |
| Protocol | `08P01` | Protocol violation |
| Internal | `XX000` | Server error |
| System | `58000` | Storage error |

## Integration

Used by:
- `raisin-server` - Main server binary with pgwire listener

Depends on:
- `raisin-sql` - SQL parsing and analysis
- `raisin-sql-execution` - Query engine
- `raisin-storage` - Storage abstraction
- `raisin-models` - Data types
- `pgwire` - Protocol implementation

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
