# Full-Text Search in RaisinDB (PostgreSQL-Compatible)

> **TODO**: Review and update this documentation to ensure accuracy with current implementation.

## Overview

RaisinDB implements PostgreSQL-compatible full-text search using `tsvector`, `tsquery`, and ranking functions. This gives you the familiar PostgreSQL API with the performance of modern full-text search engines like Tantivy.

**Key Concepts:**
- `tsvector` - Tokenized, normalized document representation
- `tsquery` - Search query with boolean operators
- `@@` - Matches operator
- `ts_rank()` / `ts_rank_cd()` - Relevance scoring

---

## Table of Contents

1. [Basic Concepts](#basic-concepts)
2. [Search Operators](#search-operators)
3. [Ranking](#ranking)
4. [Multi-Field Search](#multi-field-search)
5. [Language Support](#language-support)
6. [Advanced Queries](#advanced-queries)
7. [Integration with RaisinDB](#integration-with-raisindb)
8. [Performance](#performance)

---

## Basic Concepts

### 1. tsvector - Tokenized Document

A `tsvector` is the normalized, indexed form of text:

```sql
SELECT to_tsvector('english', 'Running faster with Rust');
-- Result: 'fast':2 'rust':4 'run':1
```

**What happened:**
- "Running" → "run" (stemmed)
- "faster" → "fast" (stemmed)
- "with" → removed (stop word)
- Numbers are word positions

### 2. tsquery - Search Query

A `tsquery` defines what to search for:

```sql
SELECT to_tsquery('english', 'rust & performance');
-- Result: 'rust' & 'perform'
```

**Operators:**
- `&` = AND
- `|` = OR
- `!` = NOT
- `<N>` = proximity (within N words)
- `:*` = prefix match

### 3. @@ - Matches Operator

The `@@` operator tests if a tsvector matches a tsquery:

```sql
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'rust & performance');
```

---

## Search Operators

### AND (`&`)

```sql
-- Both words must appear
to_tsquery('english', 'rust & performance')
```

### OR (`|`)

```sql
-- Either word can appear
to_tsquery('english', 'rust | python')
```

### NOT (`!`)

```sql
-- First word, but not second
to_tsquery('english', 'rust & !python')
```

### Complex Boolean

```sql
-- (rust OR python) AND performance
to_tsquery('english', '(rust | python) & performance')
```

### Prefix Match (`:*`)

```sql
-- Matches: perform, performance, performing, etc.
to_tsquery('english', 'perform:*')
```

### Proximity (`<N>`)

```sql
-- Words within 2 positions
to_tsquery('english', 'rust <2> performance')
```

---

## Ranking

### Basic Ranking with `ts_rank()`

```sql
SELECT
    id,
    properties ->> 'title' AS title,
    ts_rank(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'rust & code')
    ) AS rank
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'rust & code')
ORDER BY rank DESC;
```

**Returns:** Score between 0 and 1 based on:
- Term frequency
- Document length normalization
- Term position

### Coverage Density with `ts_rank_cd()`

```sql
SELECT
    id,
    properties ->> 'title' AS title,
    ts_rank_cd(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'optimize & performance')
    ) AS rank
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'optimize & performance')
ORDER BY rank DESC;
```

**More precise:** Considers how closely terms appear together.

---

## Multi-Field Search

### Search Across Multiple Fields

Use `setweight()` to boost important fields:

```sql
SELECT
    id,
    properties ->> 'title' AS title,
    ts_rank_cd(
        setweight(to_tsvector('english', properties ->> 'title'), 'A') ||
        setweight(to_tsvector('english', properties ->> 'body'), 'B'),
        to_tsquery('english', 'rust & optimize')
    ) AS rank
FROM nodes
WHERE (
    setweight(to_tsvector('english', properties ->> 'title'), 'A') ||
    setweight(to_tsvector('english', properties ->> 'body'), 'B')
) @@ to_tsquery('english', 'rust & optimize')
ORDER BY rank DESC;
```

**Weight Levels:**
- `A` = Highest (e.g., title)
- `B` = High (e.g., description)
- `C` = Medium (e.g., body)
- `D` = Low (e.g., tags)

### Three Fields Example

```sql
SELECT
    id,
    properties ->> 'title' AS title,
    ts_rank(
        setweight(to_tsvector('english', coalesce(properties ->> 'title', '')), 'A') ||
        setweight(to_tsvector('english', coalesce(properties ->> 'description', '')), 'B') ||
        setweight(to_tsvector('english', coalesce(properties ->> 'body', '')), 'C'),
        to_tsquery('english', 'database')
    ) AS rank
FROM nodes
WHERE node_type = 'my:Article'
ORDER BY rank DESC;
```

---

## Language Support

### Built-in Configurations

PostgreSQL (and RaisinDB) support multiple language configurations:

- `english` - English stemming and stop words
- `german` - German language rules
- `french` - French language rules
- `spanish` - Spanish language rules
- `simple` - No stemming, no stop words

### English Configuration

```sql
SELECT * FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'running');
-- Matches: run, running, runs
```

### Simple Configuration (No Stemming)

```sql
SELECT * FROM nodes
WHERE to_tsvector('simple', properties ->> 'body')
    @@ to_tsquery('simple', 'running');
-- Only matches exact word "running"
```

### Dynamic Language Selection

```sql
-- Use language property from node
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector(
    coalesce(properties ->> 'language', 'english'),
    properties ->> 'body'
) @@ to_tsquery(
    coalesce(properties ->> 'language', 'english'),
    'search & terms'
);
```

---

## Advanced Queries

### User-Friendly Query Parsing

#### `plainto_tsquery()` - Plain Text

Automatically converts plain text to proper tsquery:

```sql
-- User types: "rust programming language"
SELECT * FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ plainto_tsquery('english', 'rust programming language');
-- Converts to: 'rust' & 'programming' & 'language'
```

#### `phraseto_tsquery()` - Phrase Search

Treats input as an exact phrase:

```sql
SELECT * FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ phraseto_tsquery('english', 'high performance computing');
```

#### `websearch_to_tsquery()` - Google-like Syntax

Supports intuitive search syntax:

```sql
-- Supports: quotes, OR, minus sign
SELECT * FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ websearch_to_tsquery('english', 'rust OR python -javascript');
```

### Prefix Search (Autocomplete)

```sql
-- Matches: perform, performance, performing
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'perform:*');
```

### Phrase/Proximity Search

```sql
-- Words within 2 positions
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'rust <2> performance');
```

---

## Integration with RaisinDB

### Combined with Hierarchy

```sql
-- Search within specific path
SELECT
    id,
    path,
    properties ->> 'title' AS title,
    ts_rank(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'rust')
    ) AS rank
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'rust')
ORDER BY rank DESC;
```

### Search Direct Children

```sql
SELECT
    id,
    name,
    properties ->> 'title' AS title,
    ts_rank(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'database')
    ) AS rank
FROM nodes
WHERE PARENT(path) = '/content/articles'
AND to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'database')
ORDER BY rank DESC;
```

### With Property Filters

```sql
SELECT
    id,
    properties ->> 'title' AS title,
    ts_rank(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'performance')
    ) AS rank
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'performance')
AND properties ->> 'status' = 'published'
AND JSON_EXISTS(properties, '$.featured')
ORDER BY rank DESC;
```

### Pagination with Full-Text

```sql
-- Ranked results with cursor pagination
SELECT
    id,
    properties ->> 'title' AS title,
    created_at,
    ts_rank(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'rust')
    ) AS rank
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'rust')
ORDER BY rank DESC, created_at DESC, id DESC
LIMIT 20;
```

---

## Performance

### Optimal Setup (Future)

For production use, RaisinDB should support generated columns and GIN indexes:

```sql
-- 1. Add generated column for tsvector
ALTER TABLE nodes ADD COLUMN document tsvector
GENERATED ALWAYS AS (
  setweight(to_tsvector('english', properties ->> 'title'), 'A') ||
  setweight(to_tsvector('english', properties ->> 'body'), 'B')
) STORED;

-- 2. Create GIN index
CREATE INDEX idx_nodes_fts ON nodes USING GIN (document);

-- 3. Then queries are fast
SELECT * FROM nodes
WHERE document @@ to_tsquery('english', 'rust & performance')
ORDER BY ts_rank(document, to_tsquery('english', 'rust & performance')) DESC;
```

### RaisinDB Schema Approach

In RaisinDB, this maps to schema definitions:

```json
{
  "name": "body",
  "type": "String",
  "index": {
    "kind": "fulltext",
    "language": "en",
    "fields": ["title:A", "body:B"]
  }
}
```

**Behind the scenes:**
- `language: "en"` → maps to PostgreSQL `'english'` config
- Tantivy handles the actual indexing
- Queries use PostgreSQL syntax
- Results ranked like PostgreSQL

### Performance Tips

✅ **DO:**
- Use generated tsvector columns
- Create GIN indexes on tsvector
- Limit result sets with LIMIT
- Combine with path filters (`PATH_STARTS_WITH`)
- Use property filters to narrow search

❌ **DON'T:**
- Call `to_tsvector()` in SELECT for every row
- Do full-table scans without indexes
- Use LIKE instead of full-text search
- Search without any filters

---

## Debugging

### See How Text is Tokenized

```sql
SELECT to_tsvector('english', 'Running faster with Rust programming language');
-- Result: 'fast':2 'languag':6 'program':5 'run':1 'rust':4
```

### See How Query is Parsed

```sql
SELECT to_tsquery('english', 'rust & performance');
-- Result: 'rust' & 'perform'
```

### Test if Query Matches

```sql
SELECT
    'rust programming language'::text AS original,
    to_tsvector('english', 'rust programming language') AS vector,
    to_tsquery('english', 'rust & program') AS query,
    to_tsvector('english', 'rust programming language')
        @@ to_tsquery('english', 'rust & program') AS matches;
```

---

## Comparison: PostgreSQL vs Other Systems

| Feature | PostgreSQL | RaisinDB | Elasticsearch |
|---------|-----------|----------|---------------|
| **API** | SQL | SQL (Postgres-compatible) | JSON REST |
| **Query Syntax** | `to_tsquery()` | Same | Query DSL |
| **Ranking** | `ts_rank()` | Same | BM25 |
| **Index** | GIN | Tantivy | Inverted Index |
| **Language Support** | Built-in configs | Same | Analyzers |
| **Integration** | Native SQL | SQL + hierarchical | Separate system |

**RaisinDB Advantage:**
- ✅ PostgreSQL-compatible SQL
- ✅ Integrated with hierarchical data
- ✅ Same query for full-text + path filters
- ✅ No separate system to manage
- ✅ Tantivy performance

---

## Examples Summary

See `tests/sql/10_fulltext_search.sql` for 33 working examples including:

1. ✅ Basic tsvector generation
2. ✅ Simple @@ operator usage
3. ✅ Boolean queries (&, |, !)
4. ✅ Ranking with ts_rank() and ts_rank_cd()
5. ✅ Multi-field search with weights
6. ✅ Prefix search (:*)
7. ✅ Proximity search (<N>)
8. ✅ User-friendly query parsing
9. ✅ Language-specific configurations
10. ✅ Integration with RaisinDB features
11. ✅ Pagination with full-text
12. ✅ Aggregations with full-text

---

## Quick Reference

### Basic Search

```sql
SELECT * FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'rust');
```

### Ranked Search

```sql
SELECT *, ts_rank(
    to_tsvector('english', properties ->> 'body'),
    to_tsquery('english', 'rust')
) AS rank
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'rust')
ORDER BY rank DESC;
```

### Multi-Field Search

```sql
WHERE (
    setweight(to_tsvector('english', properties ->> 'title'), 'A') ||
    setweight(to_tsvector('english', properties ->> 'body'), 'B')
) @@ to_tsquery('english', 'search terms')
```

### User Input

```sql
WHERE to_tsvector('english', properties ->> 'body')
    @@ plainto_tsquery('english', :user_input)
```

---

## Summary

RaisinDB provides **PostgreSQL-compatible full-text search** with:

- ✅ Same API: `to_tsvector()`, `to_tsquery()`, `@@`, `ts_rank()`
- ✅ Same operators: `&`, `|`, `!`, `:*`, `<N>`
- ✅ Same language configs: `english`, `german`, `simple`, etc.
- ✅ Integrated with hierarchical queries
- ✅ High performance via Tantivy backend

**It's PostgreSQL full-text search, optimized for hierarchical data!** 🚀
