---
sidebar_label: "News Feed"
sidebar_position: 2
---

# News Feed: Build a Content Platform

Build a full-featured news feed application with hierarchical categories, tag relationships, and article connections. This tutorial covers the core RaisinDB concepts, then guides you through implementations in your preferred language.

:::info What You'll Learn
- Define NodeTypes with typed properties and indexes
- Model hierarchical content with path-based queries
- Create article relationships with graph queries
- Use reference indexing for tag-based filtering
- Implement Row-Level Security
:::

## Choose Your Stack

| Implementation | Framework | Source Code |
|----------------|-----------|-------------|
| [SvelteKit](./sveltekit) | SvelteKit 2 + TypeScript + pg | [news-feed](https://github.com/maravilla-labs/raisindb/tree/main/examples/demo/news-feed) |
| [Spring Boot](./spring-boot) | Spring Boot 4 + Java 21 + JDBC | [news-feed-spring](https://github.com/maravilla-labs/raisindb/tree/main/examples/demo/news-feed-spring) |
| [Laravel](./laravel) | Laravel 12 + PHP 8.2 + PDO | [news-feed-php](https://github.com/maravilla-labs/raisindb/tree/main/examples/demo/news-feed-php) |

---

# Concepts

This section covers the RaisinDB-specific patterns used across all implementations.

## NodeType Definitions

NodeTypes define the schema for your content. Here are the definitions used in the News Feed app:

```sql
-- Tag NodeType: Hierarchical tagging with visual properties
CREATE NODETYPE 'news:Tag' (
  PROPERTIES (
    label String REQUIRED LABEL 'Display Label' ORDER 1,
    icon String LABEL 'Lucide Icon Name' ORDER 2,
    color String LABEL 'Hex Color' ORDER 3
  )
  INDEXABLE
);

-- Article NodeType: Full content with tags, keywords, and publishing workflow
CREATE NODETYPE 'news:Article' (
  PROPERTIES (
    title String REQUIRED FULLTEXT LABEL 'Title' ORDER 1,
    slug String REQUIRED PROPERTY_INDEX LABEL 'URL Slug' ORDER 2,
    excerpt String LABEL 'Excerpt' ORDER 3,
    body String FULLTEXT LABEL 'Body Content' ORDER 4,
    category String PROPERTY_INDEX LABEL 'Category' ORDER 5,
    keywords Array OF String FULLTEXT LABEL 'Keywords' ORDER 6,
    tags Array OF Reference LABEL 'Tags' ORDER 7,
    featured Boolean DEFAULT false PROPERTY_INDEX LABEL 'Featured' ORDER 8,
    status String DEFAULT 'draft' PROPERTY_INDEX LABEL 'Status' ORDER 9,
    publishing_date Date PROPERTY_INDEX LABEL 'Publishing Date' ORDER 10,
    views Number DEFAULT 0 LABEL 'View Count' ORDER 11,
    author String PROPERTY_INDEX LABEL 'Author' ORDER 12,
    imageUrl String LABEL 'Image URL' ORDER 13
  )
  COMPOUND_INDEX 'idx_article_status_date' ON (
    __node_type,
    status,
    publishing_date DESC
  )
  PUBLISHABLE
  INDEXABLE
);
```

**Key features:**
- `FULLTEXT` - Enables full-text search on title, body, keywords
- `PROPERTY_INDEX` - Fast lookups on category, status, author
- `Array OF Reference` - Tags are references to Tag nodes (queryable via `REFERENCES()`)
- `COMPOUND_INDEX` - Optimized queries for published articles sorted by date
- `PUBLISHABLE` - Supports draft/published workflow
- `INDEXABLE` - Enables the reference index for tag lookups

---

## Path-Based Hierarchy

Content is organized by path, which naturally encodes the hierarchy:

```
/news/
├── articles/
│   ├── tech/                    # Category (news:Category)
│   │   ├── rust-web-2024        # Article (news:Article)
│   │   └── ai-assistants        # Article
│   └── business/                # Category
│       └── startup-trends       # Article
└── tags/
    ├── tech-stack/              # Parent tag (news:Tag)
    │   ├── rust                 # Child tag
    │   └── python               # Child tag
    └── topics/
        └── trending
```

---

## Dynamic Navigation from Content

**Your navigation is your content structure.** The category tabs aren't hardcoded—they're queried from the database at runtime.

```sql
-- Get all categories for navigation menu
SELECT id, path, name, properties
FROM social
WHERE CHILD_OF('/news/articles')
  AND node_type = 'news:Category'
ORDER BY properties ->> 'sort_order' ASC
```

This means:
- **Add a category** → Navigation updates automatically
- **Rename a category** → Navigation reflects the change
- **Reorder categories** → Just update the `sort_order` property
- **No code changes required** for content structure changes

All three implementations use this pattern—the layout/shell queries categories from the database on every request (or caches them appropriately).

---

## Hierarchical Queries

### DESCENDANT_OF (Recursive)

Find all articles across all categories:

```sql
SELECT id, path, name, properties, created_at
FROM social
WHERE DESCENDANT_OF('/news/articles')
  AND node_type = 'news:Article'
  AND properties ->> 'status' = 'published'
ORDER BY properties ->> 'publishing_date' DESC
LIMIT 20
```

### CHILD_OF (Direct Children)

Find articles in a specific category only (not subcategories):

```sql
SELECT id, path, name, properties
FROM social
WHERE CHILD_OF('/news/articles/tech')
  AND node_type = 'news:Article'
```

### JSONB Filtering

Find featured published articles:

```sql
SELECT * FROM social
WHERE DESCENDANT_OF('/news/articles')
  AND properties @> '{"featured": true, "status": "published"}'
  AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
```

---

## Reference Index (Tag Search)

The `REFERENCES` predicate uses an indexed lookup for nodes that reference another node:

```sql
-- Find articles tagged with "rust" using the reference index
SELECT id, path, name, properties
FROM social
WHERE REFERENCES('social:/news/tags/tech-stack/rust')
  AND node_type = 'news:Article'
  AND properties ->> 'status' = 'published'
ORDER BY properties ->> 'publishing_date' DESC
LIMIT 10
```

**How it works:**
- Article properties contain: `{ "tags": ["social:/news/tags/tech-stack/rust", ...] }`
- RaisinDB automatically indexes these references
- `REFERENCES()` queries the index (very fast)

---

## Graph Relationships

Articles can be connected via typed edges:

| Relationship | Description |
|--------------|-------------|
| `continues` | Part of a series |
| `corrects` | Correction/update to another article |
| `contradicts` | Opposing viewpoint |
| `similar-to` | Related content (with weight) |
| `see-also` | General reference |
| `tagged-with` | Article to tag |
| `provides-evidence-for` | Supporting evidence |

### Single Hop: Find Corrections

```sql
SELECT * FROM GRAPH_TABLE(
    MATCH (this:Article)<-[:corrects]-(correction:Article)
    WHERE this.path = '/news/articles/tech/original-post'
    COLUMNS (
        correction.id AS id,
        correction.path AS path,
        correction.name AS name,
        correction.properties AS properties
    )
) AS g
```

### Multi-Hop: Article Series

```sql
SELECT * FROM GRAPH_TABLE(
    MATCH (this)-[:continues*]->(prev)
    WHERE this.path = '/news/articles/tech/part-3'
    COLUMNS (
        prev.id AS id,
        prev.path AS path,
        prev.properties AS properties
    )
) AS g
ORDER BY (g.properties ->> 'publishing_date')::TIMESTAMP ASC
```

### 2-Hop: Shared Tags

```sql
SELECT * FROM GRAPH_TABLE(
    MATCH (this)-[:tagged-with]->(tag)<-[:tagged-with]-(other)
    WHERE this.path = '/news/articles/tech/my-article'
      AND other.path <> this.path
    COLUMNS (
        other.id AS article_id,
        other.path AS article_path,
        other.name AS article_title,
        tag.name AS shared_tag
    )
) AS g
LIMIT 10
```

---

## RELATE / UNRELATE

Create and remove relationships:

```sql
-- Create a relationship
RELATE FROM path='/news/articles/tech/correction' IN WORKSPACE 'social'
  TO path='/news/articles/tech/original' IN WORKSPACE 'social'
  TYPE 'corrects' WEIGHT 1.0

-- Remove a relationship
UNRELATE FROM path='/news/articles/tech/correction' IN WORKSPACE 'social'
  TO path='/news/articles/tech/original' IN WORKSPACE 'social'
  TYPE 'corrects'
```

---

## Row-Level Security

Set user context before queries:

```sql
-- Set user context (JWT token)
SET app.user = 'eyJhbGciOiJIUzI1NiIs...';

-- Query respects RLS policies
SELECT * FROM social
WHERE DESCENDANT_OF('/news/articles')
  AND node_type = 'news:Article';

-- Always reset after
RESET app.user;
```

---

# Architecture

## Application Flow

```
┌─────────────────────────────────────────────────────────────┐
│                    Your Application                         │
│  (SvelteKit / Spring Boot / Laravel)                       │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│   1. Layout loads categories → CHILD_OF query               │
│   2. Home shows featured/recent → DESCENDANT_OF + JSONB     │
│   3. Category page → CHILD_OF + node_type filter            │
│   4. Search → REFERENCES (tags) or FULLTEXT (keywords)      │
│   5. Article detail → path lookup + GRAPH_TABLE widgets     │
│   6. Create/edit → INSERT/UPDATE with properties JSONB      │
│   7. Link articles → RELATE command                         │
│                                                             │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                       RaisinDB                              │
│                                                             │
│  Custom SQL Extensions:                                     │
│  ├── DESCENDANT_OF, CHILD_OF (hierarchy)                   │
│  ├── REFERENCES (reference index)                          │
│  ├── GRAPH_TABLE (graph queries)                           │
│  ├── NEIGHBORS (connection lookup)                         │
│  ├── RELATE / UNRELATE (graph mutations)                   │
│  └── SET app.user (RLS context)                            │
│                                                             │
│  All accessible via standard PostgreSQL protocol!           │
└─────────────────────────────────────────────────────────────┘
```

## Page Flow

```
┌─────────────────────────────────────────────────────────────┐
│  Navigation (dynamic from CHILD_OF query)                   │
│  [Home] [Tech] [Business] [Sports] [Entertainment] [Search] │
└─────────────────────────────────────────────────────────────┘
                              │
           ┌──────────────────┼──────────────────┐
           ▼                  ▼                  ▼
    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
    │    Home     │    │  Category   │    │   Search    │
    │  DESCENDANT │    │  CHILD_OF   │    │ REFERENCES  │
    │  + featured │    │  + filter   │    │ + FULLTEXT  │
    └──────┬──────┘    └──────┬──────┘    └──────┬──────┘
           │                  │                   │
           └────────────┬─────┴───────────────────┘
                        ▼
              ┌─────────────────────┐
              │   Article Detail    │
              ├─────────────────────┤
              │ Path lookup         │
              │ ─────────────────── │
              │ GRAPH_TABLE widgets │
              │ • Series timeline   │
              │ • Related articles  │
              │ • Opposing views    │
              │ • Shared tags       │
              └─────────────────────┘
```

---

## Next Steps

Choose your implementation:

- **[SvelteKit](./sveltekit)** - Modern TypeScript with server-side rendering
- **[Spring Boot](./spring-boot)** - Enterprise Java with Thymeleaf
- **[Laravel](./laravel)** - PHP with Blade templates

Or explore the concepts further:

- [SQL Reference](/docs/access/sql/overview) - Full query documentation
- [Graph Queries](/docs/access/sql/cypher) - PGQ/Cypher patterns
- [REST API](/docs/access/rest/overview) - Alternative to SQL
