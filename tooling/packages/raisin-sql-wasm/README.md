# raisin-sql-wasm

WASM bindings for RaisinDB SQL parser validation. Provides real-time SQL validation in the browser by exposing the Rust parser and analyzer via WebAssembly.

## License

This project is licensed under the Business Source License 1.1 (BSL-1.1). See the LICENSE file in the repository root for details.

## Features

- **Real-time SQL validation** - Parse and validate SQL statements in the browser
- **DDL support** - Validates `CREATE/ALTER/DROP NODETYPE/ARCHETYPE/ELEMENTTYPE` statements
- **Standard SQL** - Validates `SELECT/INSERT/UPDATE/DELETE` statements
- **Embedded Cypher validation** - Validates Cypher queries inside `CYPHER('...')` function calls
- **Workspace support** - Register workspace names as valid table aliases via the table catalog
- **Position information** - Returns line/column positions for Monaco editor markers
- **Non-blocking** - Designed to run in a Web Worker for smooth editor performance

## API

### `validate_sql(sql: string): ValidationResult`

Validates a SQL string and returns errors with position information. Also validates embedded Cypher queries inside `CYPHER('...')` function calls.

### `validate_cypher(cypher: string): ValidationResult`

Validates a standalone Cypher query string. Useful for testing or validating Cypher queries before embedding in SQL.

```typescript
interface ValidationResult {
  success: boolean;
  errors: ValidationError[];
}

interface ValidationError {
  line: number;      // 1-based line number
  column: number;    // 1-based column number
  end_line: number;
  end_column: number;
  message: string;
  severity: 'error' | 'warning';
}
```

### `set_table_catalog(catalog: TableDef[]): void`

Sets the table catalog for workspace validation. Workspace names registered here will be recognized as valid table names in SQL queries.

```typescript
interface TableDef {
  name: string;
  columns: ColumnDef[];
}

interface ColumnDef {
  name: string;
  data_type: string;
  nullable: boolean;
}
```

### `clear_table_catalog(): void`

Clears the table catalog.

### `get_table_names(): string[]`

Returns the list of registered table names (for autocomplete).

### `get_table_columns(table_name: string): ColumnDef[] | null`

Returns column definitions for a specific table (for autocomplete).

## Building

### Prerequisites

- Rust toolchain (stable)
- wasm-pack (`cargo install wasm-pack`)

### Build

```bash
# Using the build script
./build.sh

# Or manually
wasm-pack build --target web --out-dir pkg --release
```

The built package will be in `./pkg/`.

## Usage with admin-console

The WASM module is automatically built as part of the raisin-server build process via `build.rs`. The admin-console imports it as `@raisindb/sql-wasm`.

### Web Worker Integration

The module is designed to run in a Web Worker:

```typescript
// validator.worker.ts
import init, { validate_sql, set_table_catalog } from '@raisindb/sql-wasm';

await init();

// Set workspace names as valid tables
set_table_catalog([
  { name: 'social', columns: [] },
  { name: 'default', columns: [] },
]);

// Validate SQL
const result = validate_sql('SELECT * FROM social');
```

### Monaco Editor Integration

See `packages/admin-console/src/monaco/validation/` for the full integration with Monaco editor markers.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Browser (Main Thread)                    │
│  ┌─────────────────────────────────────────────────────┐    │
│  │              Monaco Editor (SqlEditor.tsx)           │    │
│  │                                                      │    │
│  │  1. User types SQL                                   │    │
│  │  2. useWasmValidator hook sends to worker            │    │
│  │  5. Receives ValidationResult                        │    │
│  │  6. Updates Monaco markers                           │    │
│  └──────────────────────┬───────────────────────────────┘    │
│                         │                                     │
│                    postMessage                                │
│                         │                                     │
│  ┌──────────────────────▼───────────────────────────────┐    │
│  │              Web Worker (validator.worker.ts)         │    │
│  │                                                       │    │
│  │  3. Calls validate_sql()                              │    │
│  │  4. Returns ValidationResult                          │    │
│  │                                                       │    │
│  │  ┌─────────────────────────────────────────────────┐ │    │
│  │  │           raisin-sql-wasm (WASM)                 │ │    │
│  │  │                                                  │ │    │
│  │  │  - DDL Parser (nom combinators)                  │ │    │
│  │  │  - SQL Parser (sqlparser-rs)                     │ │    │
│  │  │  - Cypher Parser (raisin-cypher-parser)          │ │    │
│  │  │  - Analyzer with StaticCatalog                   │ │    │
│  │  │  - Workspace registration                        │ │    │
│  │  └─────────────────────────────────────────────────┘ │    │
│  └───────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

## How Workspace Validation Works

1. The admin-console fetches workspace names via `/api/workspaces/{repo}`
2. Workspace names are passed to the WASM via `set_table_catalog()`
3. When validating SQL, the WASM:
   - Creates a `StaticCatalog` with the default nodes schema
   - Registers each workspace name using `catalog.register_workspace()`
   - Uses the `Analyzer` for validation, which recognizes workspace names as valid table aliases

This follows the same pattern used in the server-side SQL execution.

## How Embedded Cypher Validation Works

When `validate_sql()` is called, the WASM module:

1. Validates the outer SQL statement using the SQL analyzer
2. Extracts any `CYPHER('...')` function calls and their string arguments
3. Parses each embedded Cypher query using `raisin-cypher-parser`
4. Maps Cypher error positions back to the original SQL document positions

This provides seamless validation for queries like:

```sql
SELECT * FROM CYPHER('
  MATCH (n:Person)
  WHERE n.age > 25
  RETURN n.name
')
```

If the Cypher query has syntax errors, the error position will correctly point to the line/column within the SQL document where the error occurs.

## Development

### Running Tests

```bash
# Rust unit tests
cargo test

# WASM tests (requires wasm-pack)
wasm-pack test --headless --chrome
```

### Debugging

Enable console panic hooks for better error messages:

```rust
console_error_panic_hook::set_once();
```

Errors will appear in the browser console with full Rust backtraces.
