# News Feed Demo

A full-featured news feed application demonstrating RaisinDB's hierarchical PostgreSQL database capabilities.

## Features

- **CRUD Operations**: Create, read, update, and delete articles
- **Categories**: Tech, Business, Sports, Entertainment
- **Full-text Search**: Search across article titles and content
- **Tags**: Tag articles and filter by tags
- **Featured Articles**: Mark articles as featured for homepage hero
- **View Counts**: Track article views
- **Markdown Support**: Write article content in Markdown with live preview

## Tech Stack

- **Frontend**: SvelteKit 2.x with TypeScript
- **Styling**: TailwindCSS 4.x
- **Database**: RaisinDB via PostgreSQL protocol
- **Markdown**: marked

## Prerequisites

1. A running RaisinDB instance
2. Node.js 18+

## Setup

### 1. Install dependencies

```bash
npm install
```

### 2. Set up the database

Run the setup script against your RaisinDB instance to create the folder structure and sample data:

```bash
psql "postgresql://default:raisin_N7U7POgxOh9WqZIaPC5YK1W23HlEieb9@localhost:5432/social_feed_demo_rel4" -f scripts/setup-db.sql
```

### 3. Start the development server

```bash
npm run dev
```

Visit http://localhost:5173 to view the application.

## Database Structure

### Path Hierarchy

```
/news/
├── articles/
│   ├── tech/
│   │   └── {article-slug}
│   ├── business/
│   │   └── {article-slug}
│   ├── sports/
│   │   └── {article-slug}
│   └── entertainment/
│       └── {article-slug}
```

### Article Properties

| Property | Type | Description |
|----------|------|-------------|
| title | String | Article title |
| slug | String | URL-friendly identifier |
| excerpt | String | Short summary |
| body | String | Markdown content |
| category | String | Category slug |
| tags | Array | List of tags |
| featured | Boolean | Show in featured section |
| status | String | draft or published |
| views | Number | View count |
| author | String | Author name |
| imageUrl | String | Featured image URL |

## Key RaisinDB Queries Used

### Hierarchical path queries
```sql
-- Get articles in a category
SELECT * FROM social
WHERE PATH_STARTS_WITH(path, '/news/articles/tech/')
  AND node_type = 'news:Article';
```

### JSON property filtering
```sql
-- Get featured published articles
SELECT * FROM social
WHERE properties @> '{"featured": true, "status": "published"}';

-- Filter by status
WHERE properties ->> 'status' = 'published'
```

### View count increment
```sql
UPDATE social
SET properties = jsonb_set(
  properties,
  '{views}',
  to_jsonb(COALESCE((properties ->> 'views')::int, 0) + 1)
)
WHERE id = $1;
```

### Graph Queries with GRAPH_TABLE (SQL/PGQ)

RaisinDB supports SQL/PGQ (ISO SQL:2023) for querying relationships between nodes using graph patterns.

```sql
-- Find similar articles using GRAPH_TABLE
SELECT similar.*
FROM GRAPH_TABLE(
  MATCH (source:Article)-[r:similar-to]->(target:Article)
  WHERE source.path = '/superbigshit/articles/tech/rust-web-development-2025'
  COLUMNS (
    target.path AS path,
    target.name AS title,
    r.weight AS similarity_score
  )
) AS similar
ORDER BY similar.similarity_score DESC
LIMIT 5;
```

```sql
-- Find articles that share the same tags
SELECT related.*
FROM GRAPH_TABLE(
  MATCH (a1:Article)-[r1:tagged-with]->(tag:Tag)<-[r2:tagged-with]-(a2:Article)
  WHERE a1.path = '/superbigshit/articles/tech/ai-coding-assistants'
    AND a1.id <> a2.id
  COLUMNS (
    a2.path AS related_path,
    a2.name AS related_title,
    tag.name AS shared_tag,
    COUNT(*) AS shared_tag_count
  )
) AS related
ORDER BY related.shared_tag_count DESC;
```

```sql
-- Get all articles with their tags using graph traversal
SELECT articles.*
FROM GRAPH_TABLE(
  MATCH (article:Article)-[r:tagged-with]->(tag:Tag)
  WHERE tag.path = '/superbigshit/tags/tech-stack/rust'
  COLUMNS (
    article.path,
    article.name AS title,
    article.properties->>'author' AS author,
    article.properties->>'views' AS views
  )
) AS articles;
```

## Project Structure

```
src/
├── lib/
│   ├── server/db.ts      # PostgreSQL connection
│   ├── components/       # Svelte components
│   ├── stores/           # Svelte stores
│   ├── types.ts          # TypeScript types
│   └── utils.ts          # Helper functions
└── routes/
    ├── +layout.svelte    # App layout
    ├── +page.svelte      # Home page
    ├── category/[slug]/  # Category pages
    ├── article/[id]/     # Article detail
    ├── article/new/      # Create article
    ├── search/           # Search page
    └── api/              # API endpoints
```

## Building for Production

```bash
npm run build
npm run preview
```

## License

MIT
