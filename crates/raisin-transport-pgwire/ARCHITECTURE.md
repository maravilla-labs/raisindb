# Architecture: raisin-transport-pgwire

## Overview

This document describes the internal architecture of the PostgreSQL wire protocol transport layer for RaisinDB.

## Component Diagram

```
                    ┌─────────────────────────────────────────────┐
                    │              PostgreSQL Client               │
                    │     (psql, JDBC, pgAdmin, DBeaver, etc.)    │
                    └─────────────────────┬───────────────────────┘
                                          │
                              TCP Connection (port 5432)
                                          │
┌─────────────────────────────────────────┼───────────────────────────────────────┐
│                                         ▼                                        │
│  ┌──────────────────────────────────────────────────────────────────────────┐   │
│  │                             PgWireServer<H>                               │   │
│  │  ┌─────────────────┐   ┌─────────────────┐   ┌─────────────────────────┐│   │
│  │  │   TcpListener   │   │    Semaphore    │   │  HandlerFactory (H)    ││   │
│  │  │  (bind_addr)    │   │ (max_connections│   │                        ││   │
│  │  │                 │   │    tracking)    │   │  Creates per-connection││   │
│  │  │                 │   │                 │   │  handler instances     ││   │
│  │  └────────┬────────┘   └────────┬────────┘   └────────────┬───────────┘│   │
│  │           │                     │                          │            │   │
│  │           └─────────────────────┴──────────────────────────┘            │   │
│  │                                  │                                       │   │
│  │                    tokio::spawn(process_socket(...))                     │   │
│  └──────────────────────────────────┬───────────────────────────────────────┘   │
│                                     │                                            │
│  ┌──────────────────────────────────┼───────────────────────────────────────┐   │
│  │                    Connection Handler Pipeline                            │   │
│  │                                                                           │   │
│  │  Phase 1: Startup                                                         │   │
│  │  ┌───────────────────────────────────────────────────────────────────┐   │   │
│  │  │                      RaisinAuthHandler<V,P>                        │   │   │
│  │  │                                                                    │   │   │
│  │  │   StartupMessage ──► Extract user/database ──► Request Password    │   │   │
│  │  │                                                       │            │   │   │
│  │  │   PasswordMessage ◄──────────────────────────────────┘            │   │   │
│  │  │         │                                                          │   │   │
│  │  │         ▼                                                          │   │   │
│  │  │   ┌─────────────┐    ┌─────────────────────┐    ┌──────────────┐  │   │   │
│  │  │   │ Validate    │───►│ Check pgwire_access │───►│ Create       │  │   │   │
│  │  │   │ API Key     │    │ Permission          │    │ Connection   │  │   │   │
│  │  │   │ (ApiKey-    │    │                     │    │ Context      │  │   │   │
│  │  │   │  Validator) │    │                     │    │              │  │   │   │
│  │  │   └─────────────┘    └─────────────────────┘    └──────────────┘  │   │   │
│  │  │                                                                    │   │   │
│  │  │   ConnectionContext: { tenant_id, user_id, repository,            │   │   │
│  │  │                        identity_auth, session_branch }             │   │   │
│  │  └───────────────────────────────────────────────────────────────────┘   │   │
│  │                                     │                                     │   │
│  │  Phase 2: Query Processing                                                │   │
│  │  ┌───────────────────────────────────────────────────────────────────┐   │   │
│  │  │                                                                    │   │   │
│  │  │   ┌────────────────────────┐    ┌────────────────────────────┐   │   │   │
│  │  │   │  SimpleQueryHandler    │    │  ExtendedQueryHandler      │   │   │   │
│  │  │   │                        │    │                            │   │   │   │
│  │  │   │  Query("SELECT ...")   │    │  Parse ──► Bind ──► Exec   │   │   │   │
│  │  │   │         │              │    │                            │   │   │   │
│  │  │   │         ▼              │    │  RaisinStatement {         │   │   │   │
│  │  │   │  split_statements()    │    │    sql: String,            │   │   │   │
│  │  │   │         │              │    │    param_types: Vec<Type>  │   │   │   │
│  │  │   │         ▼              │    │  }                         │   │   │   │
│  │  │   │  handle_system_query() │    │                            │   │   │   │
│  │  │   │  (SET, SHOW, USE)      │    │  bind_parameters()         │   │   │   │
│  │  │   │         │              │    │  (substitute $1, $2, ...)  │   │   │   │
│  │  │   │         ▼              │    │                            │   │   │   │
│  │  │   │  execute_query()       │    │  execute via QueryEngine   │   │   │   │
│  │  │   │                        │    │                            │   │   │   │
│  │  │   └───────────┬────────────┘    └──────────────┬─────────────┘   │   │   │
│  │  │               │                                │                  │   │   │
│  │  │               └────────────────┬───────────────┘                  │   │   │
│  │  │                                │                                  │   │   │
│  │  │                                ▼                                  │   │   │
│  │  │   ┌───────────────────────────────────────────────────────────┐  │   │   │
│  │  │   │                     QueryEngine                            │  │   │   │
│  │  │   │                 (raisin-sql-execution)                     │  │   │   │
│  │  │   │                                                            │  │   │   │
│  │  │   │   - Catalog with workspaces                                │  │   │   │
│  │  │   │   - Optional auth context (RLS)                            │  │   │   │
│  │  │   │   - Indexing engines (Tantivy, HNSW)                       │  │   │   │
│  │  │   │   - Returns RowStream                                      │  │   │   │
│  │  │   └───────────────────────────┬───────────────────────────────┘  │   │   │
│  │  │                               │                                   │   │   │
│  │  └───────────────────────────────┼───────────────────────────────────┘   │   │
│  │                                  │                                        │   │
│  │  Phase 3: Result Encoding                                                 │   │
│  │  ┌───────────────────────────────┼───────────────────────────────────┐   │   │
│  │  │                               ▼                                    │   │   │
│  │  │   ┌───────────────────────────────────────────────────────────┐   │   │   │
│  │  │   │                    ResultEncoder                           │   │   │   │
│  │  │   │                                                            │   │   │   │
│  │  │   │   infer_schema_from_rows() ──► encode_schema()            │   │   │   │
│  │  │   │                                      │                     │   │   │   │
│  │  │   │   ┌──────────────────────────────────┴──────────────────┐ │   │   │   │
│  │  │   │   │                                                      │ │   │   │   │
│  │  │   │   │   FieldFormat::Text          FieldFormat::Binary     │ │   │   │   │
│  │  │   │   │        │                           │                 │ │   │   │   │
│  │  │   │   │        ▼                           ▼                 │ │   │   │   │
│  │  │   │   │   type_mapping.rs            type_mapping_binary.rs  │ │   │   │   │
│  │  │   │   │   encode_value_text()        encode_value_binary()   │ │   │   │   │
│  │  │   │   │                                                      │ │   │   │   │
│  │  │   │   └──────────────────────────────────┬───────────────────┘ │   │   │   │
│  │  │   │                                      │                     │   │   │   │
│  │  │   │   build_query_response() ◄───────────┘                    │   │   │   │
│  │  │   │         │                                                  │   │   │   │
│  │  │   │         ▼                                                  │   │   │   │
│  │  │   │   QueryResponse { schema, row_stream }                     │   │   │   │
│  │  │   └───────────────────────────────────────────────────────────┘   │   │   │
│  │  │                               │                                    │   │   │
│  │  └───────────────────────────────┼────────────────────────────────────┘   │   │
│  │                                  │                                        │   │
│  └──────────────────────────────────┼────────────────────────────────────────┘   │
│                                     │                                            │
│                                     ▼                                            │
│                      Response sent to PostgreSQL client                          │
│                                                                                  │
└──────────────────────────────────────────────────────────────────────────────────┘
```

## Data Flow

### Authentication Flow

```
Client                          Server
  │                               │
  │──────── StartupMessage ──────►│ Contains: user, database, options
  │                               │
  │◄─── AuthenticationCleartext ──│ Request password
  │                               │
  │───────── PasswordMessage ────►│ Contains: API key
  │                               │
  │                    ┌──────────┴──────────┐
  │                    │  ApiKeyValidator    │
  │                    │  - validate_api_key │
  │                    │  - has_pgwire_access│
  │                    └──────────┬──────────┘
  │                               │
  │◄──── AuthenticationOk ────────│ Success
  │◄──── ParameterStatus... ──────│ Server params
  │◄──── ReadyForQuery ───────────│ Ready ('I' = idle)
  │                               │
```

### Simple Query Flow

```
Client                          Server
  │                               │
  │──────── Query ───────────────►│ "SELECT * FROM nodes; UPDATE..."
  │                               │
  │              ┌────────────────┴────────────────┐
  │              │  split_statements()             │
  │              │  For each statement:            │
  │              │    - handle_system_query()      │
  │              │    - OR execute_query()         │
  │              └────────────────┬────────────────┘
  │                               │
  │◄──── RowDescription ──────────│ Column metadata
  │◄──── DataRow... ──────────────│ Result rows
  │◄──── CommandComplete ─────────│ "SELECT 42"
  │◄──── ReadyForQuery ───────────│ Ready for next query
  │                               │
```

### Extended Query Flow

```
Client                          Server
  │                               │
  │──────── Parse ───────────────►│ SQL with $1, $2 placeholders
  │                               │
  │◄──── ParseComplete ───────────│
  │                               │
  │──────── Bind ────────────────►│ Parameter values + formats
  │                               │
  │◄──── BindComplete ────────────│
  │                               │
  │──────── Describe ────────────►│ (optional) Get schema
  │                               │
  │◄──── RowDescription ──────────│
  │                               │
  │──────── Execute ─────────────►│ max_rows
  │                               │
  │              ┌────────────────┴────────────────┐
  │              │  bind_parameters()              │
  │              │  substitute_params()            │
  │              │  QueryEngine.execute_batch()    │
  │              └────────────────┬────────────────┘
  │                               │
  │◄──── DataRow... ──────────────│ Binary or text format
  │◄──── CommandComplete ─────────│
  │                               │
  │──────── Sync ────────────────►│
  │                               │
  │◄──── ReadyForQuery ───────────│
  │                               │
```

## Module Dependencies

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         raisin-transport-pgwire                          │
│                                                                          │
│  ┌────────────┐                                                         │
│  │   lib.rs   │ ─────────────────────────────────────────────────┐      │
│  │            │   re-exports public API                          │      │
│  └────────────┘                                                  │      │
│        │                                                         │      │
│        ├──────────────────┬──────────────────┬──────────────────┤      │
│        ▼                  ▼                  ▼                  ▼      │
│  ┌──────────┐      ┌──────────┐      ┌──────────┐      ┌──────────┐  │
│  │ server.rs│      │ auth.rs  │      │ simple_  │      │extended_ │  │
│  │          │      │          │      │ query.rs │      │ query.rs │  │
│  │PgWireConf│      │ApiKeyVal │◄─────┤          │      │          │  │
│  │PgWireSrv │      │AuthHndlr │◄─────┤ uses auth│      │ uses auth│  │
│  └────┬─────┘      │ConnCtx   │      │ context  │      │ context  │  │
│       │            └──────────┘      └────┬─────┘      └────┬─────┘  │
│       │                                   │                  │        │
│       │                                   └────────┬─────────┘        │
│       │                                            │                  │
│       │                                            ▼                  │
│       │                                   ┌──────────────────┐        │
│       │                                   │ result_encoder.rs│        │
│       │                                   │                  │        │
│       │                                   │ ColumnInfo       │        │
│       │                                   │ ResultEncoder    │        │
│       │                                   │ infer_schema_... │        │
│       │                                   └────────┬─────────┘        │
│       │                                            │                  │
│       │                            ┌───────────────┴───────────────┐  │
│       │                            ▼                               ▼  │
│       │                   ┌──────────────────┐       ┌──────────────────┐
│       │                   │ type_mapping.rs  │       │type_mapping_     │
│       │                   │                  │       │     binary.rs    │
│       │                   │ to_pg_type()     │       │                  │
│       │                   │ encode_value_text│       │encode_value_bin  │
│       │                   │ is_null()        │       │encode_*_binary   │
│       │                   └──────────────────┘       └──────────────────┘
│       │                                                               │
│       │                                            ┌──────────────────┐│
│       │                                            │    error.rs      ││
│       └───────────────────────────────────────────►│                  ││
│                                                    │PgWireTransportErr││
│                                                    │Result<T> type    ││
│                                                    └──────────────────┘│
│                                                                        │
└────────────────────────────────────────────────────────────────────────┘
                                     │
                                     │ depends on
                                     ▼
┌────────────────────────────────────────────────────────────────────────┐
│                          External Crates                                │
│                                                                         │
│  ┌─────────────┐  ┌─────────────────┐  ┌─────────────┐  ┌───────────┐ │
│  │   pgwire    │  │ raisin-sql-exec │  │ raisin-core │  │  raisin-  │ │
│  │             │  │                 │  │             │  │  storage  │ │
│  │ Protocol    │  │ QueryEngine     │  │ Permission  │  │           │ │
│  │ Handler     │  │ Row, RowStream  │  │ Service     │  │ Workspace │ │
│  │ traits      │  │ StaticCatalog   │  │             │  │ Repo Mgmt │ │
│  └─────────────┘  └─────────────────┘  └─────────────┘  └───────────┘ │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

## Key Design Decisions

### 1. Connection Context Storage

Connection context is stored per socket address in a `HashMap<String, ConnectionContext>` protected by `RwLock`. This allows:
- Fast lookup during query execution
- Session-scoped state (branch, identity)
- Automatic cleanup on disconnect

### 2. Parameter Binding

Extended query parameters are converted to JSON values, then substituted using `raisin_sql::substitute_params()`. This ensures consistency between HTTP and pgwire transports.

### 3. Schema Inference

Two approaches for schema determination:
1. **From Results**: Infer from first row's column names and types
2. **From SQL**: Use analyzer to get schema without execution (empty results)

### 4. Binary Protocol

JDBC drivers switch to binary format after ~5 prepared statement executions. Key considerations:
- PostgreSQL epoch (2000-01-01) differs from Unix epoch
- JSONB requires version byte prefix (0x01)
- UUIDs are 16 raw bytes

### 5. System Query Handling

Common PostgreSQL queries handled specially:
- `SELECT version()` - Returns RaisinDB version
- `SET/SHOW` commands - Session configuration
- Transaction isolation queries - JDBC compatibility

## Thread Safety

All handlers are `Send + Sync`:
- `RaisinAuthHandler` uses `Arc<RwLock<HashMap>>` for contexts
- `RaisinSimpleQueryHandler` uses `Arc<S>` for storage
- `RaisinExtendedQueryHandler` uses `Arc<RaisinQueryParser>`

## Error Handling

Errors flow through two paths:
1. **Transport errors** (`PgWireTransportError`) for internal issues
2. **User errors** (`PgWireError::UserError`) sent to clients with SQLSTATE codes

The `From<PgWireTransportError> for PgWireError` conversion maps error types to appropriate SQLSTATE codes.

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
