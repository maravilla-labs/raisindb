# SDK: SQL Queries

RaisinDB supports SQL queries over node data. The SDK provides both template literal and raw query APIs.

## Setup

```typescript
const db = client.database('my-repo');
```

## Template Literal Queries

Values are automatically parameterized (safe from injection):

```typescript
const title = 'Home Page';
const result = await db.sql`SELECT * FROM content WHERE properties->>'title'::String = ${title}`;

console.log(result.columns); // ['id', 'path', 'name', 'node_type', 'properties', ...]
console.log(result.rows);    // Array of row objects
```

## Raw SQL with Parameters

Use `$1`, `$2` positional placeholders:

```typescript
const result = await db.executeSql(
  "SELECT * FROM content WHERE node_type = $1 AND properties->>'published'::String = $2",
  ['Page', 'true']
);
```

## Query Builder

For more control:

```typescript
const sql = db.getSqlQuery();

// Template literal
const result = await sql.query`SELECT * FROM content WHERE node_type = ${'Page'}`;

// Raw query
const result = await sql.execute('SELECT COUNT(*) as total FROM content');

// Without parameters (use with caution)
const result = await sql.raw('SELECT * FROM content LIMIT 10');
```

## Result Shape

```typescript
interface SqlResult {
  columns: string[];
  rows: Record<string, unknown>[];
}
```

## JSON Property Queries

When querying JSON properties with the `->>` operator, cast the key to String:

```typescript
// Correct: cast the key
await db.sql`SELECT * FROM content WHERE properties->>'email'::String = ${email}`;

// Wrong: no cast (returns empty results)
// SELECT * FROM content WHERE properties->>'email' = $1
```

## Common Patterns

```typescript
// Get all nodes of a type
const pages = await db.sql`SELECT * FROM content WHERE node_type = ${'Page'}`;

// Full-text search in properties
const results = await db.sql`
  SELECT * FROM content
  WHERE properties->>'title'::String LIKE ${'%search%'}
`;

// Count by type
const counts = await db.executeSql('SELECT node_type, COUNT(*) as count FROM content GROUP BY node_type');

// Get children of a path
const children = await db.sql`SELECT * FROM content WHERE CHILD_OF(${'/pages'})`;

// Get current user node
const user = await db.executeSql('SELECT RAISIN_CURRENT_USER() as user_node');
```
