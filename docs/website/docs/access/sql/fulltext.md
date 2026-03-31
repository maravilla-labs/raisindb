# Full-Text Search

RaisinDB provides blazing-fast full-text search powered by [Tantivy](https://github.com/quickwit-oss/tantivy), a high-performance search engine library written in Rust.

## Overview

Full-text search in RaisinDB enables you to:

- Search across node names and properties
- Support for 20+ languages with automatic stemming
- Fuzzy matching with edit distance
- Wildcard queries
- Boolean operators (AND, OR, NOT)
- Relevance scoring
- Branch-aware indexing

## Architecture

### Asynchronous Indexing

RaisinDB uses an event-driven architecture for indexing:

```
Node Create/Update/Delete
        ↓
Event Handler (enqueues job)
        ↓
Persistent Job Queue (RocksDB)
        ↓
Background Worker
        ↓
Tantivy Indexing Engine
```

When you create, update, or delete a node, an indexing job is automatically enqueued. Background workers process these jobs asynchronously, ensuring your write operations remain fast.

### Multi-Tenant Organization

Indexes are organized by tenant, repository, and branch:

```
{base_path}/
  └── {tenant_id}/
      └── {repo_id}/
          └── {branch}/
              ├── meta.json
              └── [tantivy index files]
```

Each branch maintains its own independent index, enabling:
- Fast branch operations (copy-on-write)
- Branch isolation
- Per-branch search results

### Multi-Language Support

Documents can be indexed in multiple languages simultaneously. Each language variant is stored as a separate document in the index with language-specific stemming applied.

## Supported Languages

Tantivy provides stemming support for:

- English, German, French, Spanish, Italian, Portuguese
- Russian, Arabic, Danish, Dutch, Finnish, Hungarian
- Norwegian, Romanian, Swedish, Turkish

The indexer automatically applies the appropriate stemming algorithm based on the document's language.

## Index Schema

Each indexed document contains:

### Identifiers (Stored & Indexed)
- `doc_id` - Unique document identifier
- `node_id` - Node identifier
- `workspace_id` - Workspace/branch identifier
- `language` - Language code (e.g., 'en', 'de', 'fr')
- `path` - Hierarchical path
- `node_type` - Node type identifier

### Metadata
- `revision` - Revision number (uint64)
- `created_at` - Creation timestamp
- `updated_at` - Last update timestamp

### Content Fields (Full-Text Indexed)
- `name` - Node name
- `content` - Aggregated text content from properties

The `content` field automatically aggregates searchable text from node properties based on the node type schema.

## Query Syntax

### Basic Search

```
rust programming
```

Finds documents containing both "rust" and "programming" (default AND behavior).

### Boolean Operators

```
rust AND programming
rust OR python
rust NOT javascript
(rust OR go) AND performance
```

Combine terms with `AND`, `OR`, and `NOT` operators.

### Phrase Queries

```
"web development"
"machine learning"
```

Search for exact phrases.

### Wildcard Queries

Use `*` to match zero or more characters, or `?` to match a single character:

```
optim*        # Matches: optimize, optimization, optimizing
perform*      # Matches: perform, performance, performing
log?          # Matches: logo, logs, loge
```

Wildcards are converted to regex patterns internally.

### Fuzzy Search

RaisinDB supports fuzzy matching with an edit distance of 1:

```
perfomance~   # Matches: performance
databse~      # Matches: database
```

Fuzzy search is automatically applied during query parsing for single-term typos.

### Field-Specific Search

```
title:rust
author:smith
name:optimization
```

Search within specific fields.

## Search API

### Query Structure

```rust
pub struct FullTextSearchQuery {
    pub tenant_id: String,
    pub repo_id: String,
    pub workspace_id: String,  // branch name
    pub branch: String,
    pub language: String,
    pub query: String,         // Tantivy query syntax
    pub limit: usize,          // Max results to return
}
```

### Response Format

```rust
pub struct FullTextSearchResult {
    pub node_id: String,
    pub score: f32,  // Relevance score (higher = more relevant)
}
```

Results are sorted by relevance score (BM25 algorithm).

### REST API Example

```http
POST /api/fulltext/search
Content-Type: application/json

{
  "tenant_id": "tenant-1",
  "repo_id": "my-repo",
  "workspace_id": "main",
  "branch": "main",
  "language": "en",
  "query": "rust performance optimization",
  "limit": 20
}
```

**Response:**

```json
{
  "results": [
    {
      "node_id": "article-123",
      "score": 4.521
    },
    {
      "node_id": "article-456",
      "score": 3.891
    }
  ]
}
```

## Indexing Job Types

The indexer handles three types of jobs:

### AddNode / UpdateNode

Indexes a new node or re-indexes an updated node.

- Extracts searchable content from properties
- Creates one document per language variant
- Updates existing documents if already indexed

### DeleteNode

Removes a node from the index.

- Deletes all language variants
- Cleans up stale index entries

### BranchCreated

Copies the index when a new branch is created.

- Efficient copy-on-write
- Isolates branch changes
- Enables independent branch search

## Performance Characteristics

### Indexing Performance

- **Asynchronous:** Write operations return immediately
- **Batched:** Multiple jobs can be processed together
- **Incremental:** Only changed nodes are re-indexed

### Search Performance

- **Inverted Index:** O(1) term lookup
- **BM25 Ranking:** Efficient relevance scoring
- **Prefix Scan:** Fast wildcard queries
- **In-Memory Structures:** Hot indexes cached by Tantivy

Typical search latency: **< 10ms** for millions of documents.

## Configuration

### Node Type Schema

Define which properties should be indexed:

```rust
pub struct NodeTypeProperty {
    pub name: String,
    pub property_type: PropertyType,
    pub fulltext_indexed: bool,  // Include in full-text index
    // ...
}
```

Only properties marked with `fulltext_indexed: true` are included in the search index.

### Indexing Strategy

RaisinDB uses a **background worker pool** to process indexing jobs:

1. Events are captured during transactions
2. Jobs are persisted to a RocksDB queue
3. Workers poll the queue and process jobs
4. Failed jobs are retried with backoff

This ensures indexing is durable and eventually consistent.

## Branch-Aware Search

### Search Within a Branch

```http
POST /api/fulltext/search
{
  "workspace_id": "feature-branch",
  "branch": "feature-branch",
  "query": "new feature"
}
```

Only returns results from the specified branch.

### Branch Operations

When you create a new branch:

1. A `BranchCreated` job is enqueued
2. The parent branch's index is copied
3. Changes in the new branch update its index independently

This enables:
- Isolated feature development
- Preview environments with independent search
- Efficient branch merging

## Combining Full-Text with SQL

While full-text search is currently accessed via a separate API, you can combine it with SQL queries:

**Workflow:**

1. Perform full-text search to get node IDs
2. Use SQL to fetch full node data and apply additional filters

**Example:**

```typescript
// Step 1: Full-text search
const searchResults = await fetch('/api/fulltext/search', {
  method: 'POST',
  body: JSON.stringify({
    tenant_id: 'tenant-1',
    repo_id: 'my-repo',
    workspace_id: 'main',
    branch: 'main',
    language: 'en',
    query: 'rust performance',
    limit: 100
  })
});

const nodeIds = searchResults.results.map(r => r.node_id);

// Step 2: SQL query for full data
const sql = `
  SELECT id, name, path, properties
  FROM nodes
  WHERE id IN (${nodeIds.map(id => `'${id}'`).join(',')})
    AND properties ->> 'status' = 'published'
  ORDER BY created_at DESC
`;
```

This hybrid approach gives you:
- Fast relevance-based search
- Rich SQL filtering and sorting
- Complete node data retrieval

## Best Practices

### Indexing

1. **Mark relevant properties** - Set `fulltext_indexed: true` only for text properties that should be searchable
2. **Use appropriate languages** - Set the correct language for stemming accuracy
3. **Keep content concise** - Aggregate only essential text in the `content` field
4. **Monitor queue depth** - Ensure indexing keeps up with write load

### Querying

1. **Use specific queries** - More specific queries return better results
2. **Limit result sets** - Use reasonable `limit` values (e.g., 20-100)
3. **Combine with filters** - Use SQL for additional filtering after search
4. **Leverage wildcards** - Use `*` for prefix matching (e.g., `optim*`)
5. **Apply fuzzy search** - Add `~` for typo tolerance when needed

### Performance

1. **Pre-filter when possible** - Narrow search scope by branch/language
2. **Cache popular queries** - Cache search results for common searches
3. **Paginate results** - Don't fetch all matches at once
4. **Monitor index size** - Large indexes may need partitioning

## Limitations & Future Work

### Current Limitations

- SQL integration not yet available (use REST API)
- No phrase proximity search
- Boolean operators have standard precedence (use parentheses)
- Index updates are asynchronous (eventual consistency)

### Planned Features

- [ ] Direct SQL full-text queries (`WHERE FULLTEXT_MATCH(query)`)
- [ ] Phrase proximity search (`"word1 word2"~5`)
- [ ] Faceted search
- [ ] Highlighting of matched terms
- [ ] Custom ranking formulas
- [ ] Index optimization tools

## Troubleshooting

### Indexing Issues

**Symptom:** New nodes don't appear in search results

**Solutions:**
1. Check if indexing jobs are being processed (monitor queue)
2. Verify node type has `fulltext_indexed` properties
3. Check background workers are running
4. Look for errors in indexing logs

### Search Quality

**Symptom:** Irrelevant results returned

**Solutions:**
1. Use more specific queries
2. Add field-specific searches (`title:keyword`)
3. Combine with boolean operators
4. Use exact phrases for precision
5. Filter results with SQL post-search

### Performance

**Symptom:** Slow search queries

**Solutions:**
1. Reduce result `limit`
2. Use more specific queries to narrow matches
3. Pre-filter by branch/language
4. Check index size and consider partitioning
5. Ensure indexes are on fast storage (SSD)

## Examples

### Simple Text Search

```json
{
  "query": "database performance",
  "limit": 10
}
```

### Wildcard Search

```json
{
  "query": "optim* AND perform*",
  "limit": 20
}
```

### Field-Specific Search

```json
{
  "query": "title:rust AND author:smith",
  "limit": 15
}
```

### Complex Boolean Query

```json
{
  "query": "(rust OR python) AND (performance OR optimization) NOT deprecated",
  "limit": 25
}
```

### Multi-Language Search

```typescript
// Search in English
const enResults = await search({ language: 'en', query: 'database' });

// Search in German
const deResults = await search({ language: 'de', query: 'datenbank' });
```

## What's Next?

- [Query Examples](examples.md) - Combining full-text with SQL
- [RaisinSQL Reference](raisinsql.md) - SQL query syntax
- [REST API Overview](../rest/overview.md) - HTTP endpoint catalog
