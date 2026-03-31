# JSON Operations in RaisinDB

> **TODO**: Review and update this documentation to ensure accuracy with current implementation.

## Postgres-Compatible JSONB Handling

RaisinDB follows PostgreSQL's approach to JSONB columns. You **do not need** special functions to work with JSON - you can select JSONB columns directly and use standard PostgreSQL operators.

---

## Direct Column Selection

### ✅ Correct (Postgres-style)

```sql
-- Select entire JSONB column directly
SELECT properties FROM nodes WHERE id = 'node-123';

-- Select multiple columns including JSONB
SELECT id, name, properties, translations FROM nodes;

-- Select with JSON operators
SELECT
    id,
    properties,
    properties ->> 'title' AS title
FROM nodes;
```

### ❌ Not Needed

```sql
-- ❌ TO_JSON() is NOT needed in RaisinDB
SELECT TO_JSON(properties) FROM nodes;  -- Don't do this

-- ✅ Just select the column directly
SELECT properties FROM nodes;  -- Do this instead
```

---

## JSON Operators (Postgres-Compatible)

### 1. Arrow Operators (`->` and `->>`)

```sql
-- ->> extracts as TEXT
SELECT properties ->> 'title' AS title FROM nodes;

-- -> extracts as JSONB (for nested access)
SELECT properties -> 'metadata' ->> 'author' AS author FROM nodes;
```

**Difference:**
- `->` returns JSONB (use for chaining)
- `->>` returns TEXT (use for final extraction)

### 2. Containment Operator (`@>`)

```sql
-- Check if JSON contains specific key-value
SELECT * FROM nodes
WHERE properties @> '{"status": "published"}';

-- Check if array contains value
SELECT * FROM nodes
WHERE properties @> '{"tags": ["rust"]}';

-- Check nested structure
SELECT * FROM nodes
WHERE properties @> '{"metadata": {"featured": true}}';
```

### 3. Existence Operator (`?`)

```sql
-- Check if top-level key exists (Postgres standard)
SELECT * FROM nodes WHERE properties ? 'title';

-- Check if any of multiple keys exist
SELECT * FROM nodes WHERE properties ?| array['title', 'name'];

-- Check if all keys exist
SELECT * FROM nodes WHERE properties ?& array['title', 'status'];
```

---

## JSON Functions

### JSON_VALUE() - Extract with Type Casting

```sql
-- Extract as TEXT (default)
SELECT JSON_VALUE(properties, '$.title') AS title FROM nodes;

-- Extract as DOUBLE
SELECT JSON_VALUE(properties, '$.price' RETURNING DOUBLE) AS price
FROM nodes;

-- Use in WHERE clause
SELECT * FROM nodes
WHERE JSON_VALUE(properties, '$.price' RETURNING DOUBLE) > 100.0;

-- Use in ORDER BY
SELECT * FROM nodes
ORDER BY JSON_VALUE(properties, '$.views' RETURNING DOUBLE) DESC;
```

**Supported Types:**
- `RETURNING DOUBLE` - numeric values
- `RETURNING INTEGER` - integer values
- Default (no RETURNING) - text values

### JSON_EXISTS() - Check Path Existence

```sql
-- Check if path exists
SELECT * FROM nodes
WHERE JSON_EXISTS(properties, '$.seo.title');

-- Check nested path
SELECT * FROM nodes
WHERE JSON_EXISTS(properties, '$.metadata.social.twitter');

-- Combine with other conditions
SELECT * FROM nodes
WHERE JSON_EXISTS(properties, '$.featured')
AND properties @> '{"featured": true}';
```

---

## Common Patterns

### 1. Extract Multiple Properties

```sql
SELECT
    id,
    name,
    properties ->> 'title' AS title,
    properties ->> 'status' AS status,
    properties ->> 'author' AS author,
    JSON_VALUE(properties, '$.views' RETURNING DOUBLE) AS views
FROM nodes;
```

### 2. Filter by JSON Property

```sql
-- Simple equality
SELECT * FROM nodes
WHERE properties ->> 'status' = 'published';

-- Range query on numeric property
SELECT * FROM nodes
WHERE JSON_VALUE(properties, '$.price' RETURNING DOUBLE) BETWEEN 10.0 AND 100.0;

-- Check if property exists and has value
SELECT * FROM nodes
WHERE JSON_EXISTS(properties, '$.featured')
AND properties @> '{"featured": true}';
```

### 3. Aggregations with JSON

```sql
-- Group by JSON property
SELECT
    properties ->> 'category' AS category,
    COUNT(*) AS count
FROM nodes
WHERE properties ->> 'status' = 'published'
GROUP BY properties ->> 'category';

-- Average of numeric JSON property
SELECT AVG(JSON_VALUE(properties, '$.price' RETURNING DOUBLE)) AS avg_price
FROM nodes
WHERE node_type = 'my:Product';
```

### 4. Sorting by JSON Property

```sql
-- Sort by text property
SELECT * FROM nodes
ORDER BY properties ->> 'title';

-- Sort by numeric property
SELECT * FROM nodes
ORDER BY JSON_VALUE(properties, '$.views' RETURNING DOUBLE) DESC;

-- Multiple sort columns
SELECT * FROM nodes
ORDER BY
    properties ->> 'category',
    JSON_VALUE(properties, '$.priority' RETURNING DOUBLE) DESC;
```

---

## JSONPath Syntax

RaisinDB supports standard JSONPath expressions:

```sql
-- Root level: $.property_name
JSON_VALUE(properties, '$.title')

-- Nested: $.path.to.property
JSON_VALUE(properties, '$.metadata.author.name')

-- Array access: $.array[0]
JSON_VALUE(properties, '$.tags[0]')

-- Nested in array: $.items[0].price
JSON_VALUE(properties, '$.items[0].price')
```

---

## Performance Tips

### ✅ DO

1. **Use operators for simple extraction**
   ```sql
   -- Fast
   WHERE properties ->> 'status' = 'published'
   ```

2. **Use @> for containment checks**
   ```sql
   -- Efficient
   WHERE properties @> '{"status": "published"}'
   ```

3. **Index on extracted values** (if supported)
   ```sql
   -- Create functional index (future)
   CREATE INDEX idx_status ON nodes ((properties ->> 'status'));
   ```

### ❌ DON'T

1. **Don't use LIKE on entire JSON**
   ```sql
   -- ❌ Slow - scans entire JSON as text
   WHERE properties::TEXT LIKE '%published%'

   -- ✅ Better - use specific operators
   WHERE properties ->> 'status' = 'published'
   ```

2. **Don't extract in SELECT for filtering**
   ```sql
   -- ❌ Inefficient
   SELECT * FROM (
       SELECT *, properties ->> 'status' AS status FROM nodes
   ) WHERE status = 'published'

   -- ✅ Better - filter directly
   SELECT * FROM nodes WHERE properties ->> 'status' = 'published'
   ```

---

## Comparison: Postgres vs RaisinDB

| Feature | PostgreSQL | RaisinDB |
|---------|-----------|----------|
| **Direct column select** | ✅ `SELECT properties` | ✅ Same |
| **`->` operator** | ✅ JSONB extraction | ✅ Same |
| **`->>` operator** | ✅ Text extraction | ✅ Same |
| **`@>` operator** | ✅ Contains | ✅ Same |
| **`?` operator** | ✅ Key exists | ✅ Same |
| **JSON_VALUE()** | ✅ With type casting | ✅ Same |
| **JSON_EXISTS()** | ✅ Path checking | ✅ Same |
| **TO_JSON()** | ⚠️ For converting rows | ❌ Not needed |
| **JSONB_*() functions** | ✅ Many helpers | ⚠️ Subset supported |

### Key Differences

**PostgreSQL has:**
- `TO_JSON()` - Converts row to JSON
- `ROW_TO_JSON()` - Converts row to JSON
- `JSONB_BUILD_OBJECT()` - Constructs JSON
- `JSONB_AGG()` - Aggregates to JSON array
- Many more JSONB functions

**RaisinDB focuses on:**
- ✅ Core operators (`->`, `->>`, `@>`, `?`)
- ✅ Essential functions (`JSON_VALUE`, `JSON_EXISTS`)
- ✅ Direct column selection (Postgres-style)
- ✅ Standard JSONPath syntax

**Why?** RaisinDB prioritizes the most commonly used JSONB operations that work efficiently with hierarchical data and RocksDB storage.

---

## Examples from Test Suite

See `tests/sql/03_json_operations.sql` for 16 working examples including:

- ✅ JSON extraction with `->>` operator
- ✅ JSON containment with `@>` operator
- ✅ `JSON_VALUE()` with type casting
- ✅ `JSON_EXISTS()` for path checking
- ✅ Combining JSON with hierarchy functions
- ✅ JSON aggregations
- ✅ Complex filtering and sorting
- ✅ Direct JSONB column selection (no TO_JSON needed)

---

## Summary

**In RaisinDB, treat JSONB like PostgreSQL:**

1. ✅ Select columns directly: `SELECT properties FROM nodes`
2. ✅ Use `->>` for text extraction: `properties ->> 'title'`
3. ✅ Use `@>` for containment: `properties @> '{"status": "published"}'`
4. ✅ Use `JSON_VALUE()` for typed extraction: `JSON_VALUE(properties, '$.price' RETURNING DOUBLE)`
5. ❌ Don't use `TO_JSON()` - not needed for JSONB columns

**It's Postgres-compatible by design!** 🐘
