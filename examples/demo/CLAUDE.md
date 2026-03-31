# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is the **news-feed demo** application for RaisinDB - a SvelteKit application demonstrating RaisinDB's hierarchical PostgreSQL database capabilities.

## Development Commands

```bash
# Navigate to the app directory first
cd news-feed

# Install dependencies
npm install

# Run development server (http://localhost:5173)
npm run dev

# Type checking
npm run check

# Build for production
npm run build
```

## Architecture

**Tech Stack**: SvelteKit 2.x + Svelte 5 + TypeScript + TailwindCSS 4.x + RaisinDB (PostgreSQL protocol)

**Key paths**:
- `news-feed/src/lib/server/db.ts` - PostgreSQL connection pool and query helpers
- `news-feed/src/lib/types.ts` - TypeScript types and path utilities (BASE_PATH = `/superbigshit`)
- `news-feed/src/routes/` - SvelteKit routes with server-side data loading

**Database queries use workspace name `social`** (not `default`).

## RaisinDB Query Patterns

Use `DESCENDANT_OF()` and `CHILD_OF()` without the path parameter format:
```sql
-- CORRECT
WHERE DESCENDANT_OF('/superbigshit/articles')
WHERE CHILD_OF('/superbigshit/articles')

-- WRONG (do not use function-style with path parameter)
WHERE CHILD_OF(path, '/superbigshit/articles')
```

### REFERENCES() - Query by Reference Index

Use `REFERENCES()` to find nodes that reference a specific target path. References are stored as JSON objects with `raisin:ref`, `raisin:workspace`, and `raisin:path` keys.

```sql
-- Find all articles that reference a specific tag
-- Format: REFERENCES('workspace:/path')
SELECT * FROM social
WHERE REFERENCES('social:/superbigshit/tags/tech-stack/rust')
  AND node_type = 'news:Article';

-- The reference target format is: workspace:path
-- - workspace: name without leading slash (e.g., 'social', 'default')
-- - path: must start with slash (e.g., '/superbigshit/tags/rust')

-- References are stored in properties as JSON arrays:
-- "tags": [
--   {"raisin:ref": "tag-id", "raisin:workspace": "social", "raisin:path": "/superbigshit/tags/tech-stack/rust"},
--   {"raisin:ref": "tag-id2", "raisin:workspace": "social", "raisin:path": "/superbigshit/tags/topics/trending"}
-- ]
```

JSON property access:
```sql
-- Filter by property value
WHERE properties ->> 'status'::TEXT = 'published'

-- JSONB containment for multiple conditions
WHERE properties @> '{"featured": true, "status": "published"}'

-- Array containment check
WHERE properties -> 'tags'::TEXT ? $1 = true
```

### RELATE / UNRELATE - Manage Node Relations

Use `RELATE` and `UNRELATE` to create and remove directed relationships between nodes. Relations support custom types, weights, cross-workspace connections, and branch-specific operations.

```sql
-- RELATE creates a directed relationship between two nodes
-- Syntax: RELATE [IN BRANCH 'branch'] FROM <source> TO <target> [TYPE 'type'] [WEIGHT number]
-- Node references: path='/path' or id='uuid'

-- Basic relation (defaults to type 'references')
RELATE FROM path='/content/blog/post1' TO path='/content/blog/post2';

-- Relation by node ID
RELATE FROM id='abc123' TO id='def456';

-- With custom relation type
RELATE FROM path='/content/articles/a1' TO path='/content/articles/a2' TYPE 'see-also';
RELATE FROM path='/products/laptop' TO path='/products/case' TYPE 'accessory';
RELATE FROM path='/users/alice' TO path='/users/bob' TYPE 'follows';

-- With weight for ranking/scoring
RELATE FROM path='/content/post1' TO path='/content/post2' WEIGHT 0.8;
RELATE FROM path='/content/article1' TO path='/content/article2' TYPE 'similarity' WEIGHT 0.95;

-- Cross-workspace relations
RELATE
  FROM path='/content/blog/post1' IN WORKSPACE 'website'
  TO path='/products/item1' IN WORKSPACE 'ecommerce';

-- Branch-specific relation
RELATE IN BRANCH 'feature-branch'
  FROM path='/content/draft1'
  TO path='/content/draft2'
  TYPE 'related';
```

```sql
-- UNRELATE removes relationships between nodes
-- Syntax: UNRELATE [IN BRANCH 'branch'] FROM <source> TO <target> [TYPE 'type']

-- Remove all relation types between nodes
UNRELATE FROM path='/content/blog/post1' TO path='/content/blog/post2';

-- Remove only a specific relation type
UNRELATE FROM path='/content/post1' TO path='/content/post2' TYPE 'related-to';
UNRELATE FROM path='/users/alice' TO path='/users/bob' TYPE 'follows';

-- Remove by node ID
UNRELATE FROM id='abc123' TO id='def456';

-- Cross-workspace removal
UNRELATE
  FROM path='/content/blog/post1' IN WORKSPACE 'website'
  TO path='/products/item1' IN WORKSPACE 'ecommerce';

-- Branch-specific removal
UNRELATE IN BRANCH 'feature-branch'
  FROM path='/content/draft1'
  TO path='/content/draft2';
```

**Common use cases:**
- Content recommendations (`TYPE 'related-to'`, `TYPE 'see-also'`)
- Product accessories (`TYPE 'accessory'`, `TYPE 'bundle-item'`)
- Social connections (`TYPE 'follows'`, `TYPE 'likes'`)
- Document navigation (`TYPE 'next'`, `TYPE 'previous'`)
- Content similarity with scoring (`WEIGHT 0.0-1.0`)

### NEIGHBORS() - Simple Graph Traversal

Use `NEIGHBORS()` for simple single-hop graph traversals. It's a table-valued function that returns connected nodes.

```sql
-- Syntax: NEIGHBORS(start_node, direction, relation_type)
-- start_node: 'workspace:/path' or node ID
-- direction: 'OUT' (outgoing), 'IN' (incoming), 'BOTH' (bidirectional)
-- relation_type: filter by type (or NULL for all types)

-- Get outgoing 'tagged-with' relations
SELECT n.id, n.path, n.name, n.relation_type, n.weight
FROM NEIGHBORS('social:/superbigshit/articles/tech/rust-web-development-2025', 'OUT', 'tagged-with') AS n;

-- Get incoming 'continues' relations (articles that continue this one)
SELECT n.id, n.path, n.properties
FROM NEIGHBORS('social:/superbigshit/articles/tech/ai-coding-assistants', 'IN', 'continues') AS n
WHERE n.node_type = 'news:Article';

-- Get all outgoing relations (NULL = no type filter)
SELECT n.id, n.path, n.relation_type, n.weight
FROM NEIGHBORS('social:/superbigshit/articles/tech/rust-web-development-2025', 'OUT', NULL) AS n
WHERE n.relation_type IN ('similar-to', 'see-also', 'updates')
ORDER BY n.weight DESC;

-- Bidirectional traversal for symmetric relations
SELECT n.id, n.path
FROM NEIGHBORS('social:/superbigshit/articles/tech/ai-coding-assistants', 'BOTH', 'contradicts') AS n
WHERE n.node_type = 'news:Article';
```

**NEIGHBORS columns available:**
- `id` - Node UUID
- `path` - Node path
- `name` - Node name
- `node_type` - Node type
- `properties` - Node properties (JSONB)
- `relation_type` - The relationship type
- `weight` - Relationship weight (0.0-1.0)
- `created_at`, `updated_at` - Timestamps

**When to use NEIGHBORS vs GRAPH_TABLE:**
- **NEIGHBORS**: Simple single-hop traversals, filtering by relation type
- **GRAPH_TABLE**: Multi-hop patterns, complex path matching, 2+ hop patterns like `(a)-[:X]->(b)<-[:Y]-(c)`

---

## Property Graph Queries (SQL/PGQ - ISO SQL:2023)

RaisinDB implements SQL/PGQ (ISO SQL:2023 Part 16) for property graph pattern matching.
Use `GRAPH_TABLE` to query relationships between nodes using graph patterns.

### Basic Structure

Every GRAPH_TABLE query has three parts:

```sql
SELECT columns FROM GRAPH_TABLE(
    MATCH pattern                   -- Required: the graph pattern to find
    [WHERE condition]               -- Optional: filter the matches
    COLUMNS (output_columns)        -- Required: what to return
) [AS alias]
```

### Node Patterns

Nodes are written in parentheses `()`. Assign a variable name and filter by label (node_type).

| Pattern | Meaning |
|---------|---------|
| `(n)` | Any node, named `n` |
| `(n:User)` | Node with label `User` |
| `(n:User\|Admin)` | Node with label `User` OR `Admin` |
| `()` | Anonymous node (no variable) |

### Relationship Patterns

| Pattern | Direction | Meaning |
|---------|-----------|---------|
| `-[r]->` | Right (outgoing) | From left node to right node |
| `<-[r]-` | Left (incoming) | From right node to left node |
| `-[r]-` | Any direction | Either direction matches |

| Pattern | Meaning |
|---------|---------|
| `-[r]->` | Any relationship type |
| `-[r:FOLLOWS]->` | Only FOLLOWS relationships |
| `-[r:FOLLOWS\|LIKES]->` | FOLLOWS or LIKES relationships |
| `-[:FOLLOWS]->` | FOLLOWS (anonymous, no variable needed) |

### Multi-Hop Patterns (Variable-Length Paths)

| Pattern | Hops | Meaning |
|---------|------|---------|
| `-[:FOLLOWS*]->` | 1-10 | Default: 1 to 10 hops |
| `-[:FOLLOWS*3]->` | Exactly 3 | Exactly 3 hops |
| `-[:FOLLOWS*1..3]->` | 1-3 | Between 1 and 3 hops |
| `-[:FOLLOWS*2..]->` | 2-10 | At least 2 hops (max 10) |
| `-[:FOLLOWS*..5]->` | 1-5 | At most 5 hops |

### System Fields on Nodes

| Field | Description |
|-------|-------------|
| `node.id` | Node UUID |
| `node.workspace` | Workspace identifier |
| `node.node_type` | Node type/label |
| `node.path` | Hierarchical path |
| `node.name` | Node name |
| `node.parent_id` | Parent node UUID |
| `node.created_at` | Creation timestamp |
| `node.updated_at` | Last update timestamp |

---

### Example Queries

#### Find Related Articles (similar-to relationships)

```sql
SELECT * FROM GRAPH_TABLE(
    MATCH (a:Article)-[r:`similar-to`]->(b:Article)
    WHERE a.path = '/superbigshit/articles/tech/rust-web-development-2025'
    COLUMNS (
        b.id AS related_id,
        b.path AS related_path,
        b.name AS related_title,
        r.weight AS similarity_score
    )
) AS related
ORDER BY similarity_score DESC;
```

#### Find Articles by Tag (tagged-with relationships)

```sql
SELECT * FROM GRAPH_TABLE(
    MATCH (article:Article)-[:tagged-with]->(tag:Tag)
    WHERE tag.path = '/superbigshit/tags/tech-stack/rust'
    COLUMNS (
        article.id,
        article.path,
        article.name AS title
    )
) AS rust_articles;
```

#### Find All Tags for an Article

```sql
SELECT * FROM GRAPH_TABLE(
    MATCH (article:Article)-[:tagged-with]->(tag:Tag)
    WHERE article.path = '/superbigshit/articles/tech/ai-coding-assistants'
    COLUMNS (
        tag.path,
        tag.name AS tag_name
    )
) AS article_tags;
```

#### Multi-Hop: Find Articles 2 Hops Away

```sql
SELECT * FROM GRAPH_TABLE(
    MATCH (start:Article)-[:`similar-to`*2]->(distant:Article)
    WHERE start.path = '/superbigshit/articles/tech/rust-web-development-2025'
    COLUMNS (
        distant.id,
        distant.path,
        distant.name AS title
    )
) AS distant_articles;
```

#### Find Who Follows Who (Social)

```sql
SELECT * FROM GRAPH_TABLE(
    MATCH (a:User)-[:FOLLOWS]->(b:User)
    WHERE a.name = 'alice'
    COLUMNS (b.name AS followed_user)
);
```

#### Content Recommendations with Scores

```sql
SELECT
    g.related_id,
    g.related_title,
    g.similarity_score,
    n.properties ->> 'excerpt' AS excerpt
FROM GRAPH_TABLE(
    MATCH (source:Article)-[r:`similar-to`]->(target:Article)
    WHERE source.path = '/superbigshit/articles/tech/rust-web-development-2025'
    COLUMNS (
        target.id AS related_id,
        target.name AS related_title,
        r.weight AS similarity_score
    )
) AS g
JOIN social n ON n.id = g.related_id
WHERE n.properties ->> 'status' = 'published'
ORDER BY g.similarity_score DESC
LIMIT 5;
```

#### Find Influencers (Most Followed)

```sql
SELECT * FROM GRAPH_TABLE(
    MATCH (follower:User)-[:FOLLOWS]->(influencer:User)
    COLUMNS (
        influencer.name,
        influencer.id,
        COUNT(*) AS follower_count
    )
)
ORDER BY follower_count DESC
LIMIT 10;
```

#### Content Tree Traversal

```sql
-- Find all content under a folder, any depth
SELECT * FROM GRAPH_TABLE(
    MATCH (root:Folder)-[:CONTAINS*1..]->(content)
    WHERE root.path = '/content/website'
    COLUMNS (
        content.id,
        content.path,
        content.node_type,
        content.name
    )
)
ORDER BY content.path;
```

---

### Integration with Standard SQL

GRAPH_TABLE returns a table, so you can use it anywhere a table is expected:

#### With JOINs

```sql
SELECT g.user_name, n.properties->>'email' AS email
FROM GRAPH_TABLE(
    MATCH (u:User)-[:FOLLOWS*2]->(fof:User)
    WHERE u.name = 'alice'
    COLUMNS (fof.id AS user_id, fof.name AS user_name)
) AS g
JOIN nodes n ON n.id = g.user_id
WHERE n.properties->>'verified' = 'true';
```

#### With CTEs

```sql
WITH influencers AS (
    SELECT * FROM GRAPH_TABLE(
        MATCH (f:User)-[:FOLLOWS]->(i:User)
        COLUMNS (i.id, i.name, COUNT(*) AS followers)
    )
    WHERE followers > 1000
)
SELECT * FROM influencers ORDER BY followers DESC;
```

#### In Subqueries

```sql
SELECT * FROM social
WHERE id IN (
    SELECT user_id FROM GRAPH_TABLE(
        MATCH (influencer:User)<-[:FOLLOWS*1..2]-(follower:User)
        WHERE influencer.name = 'celebrity'
        COLUMNS (follower.id AS user_id)
    )
);
```

---

### GeoJSON / Geospatial Functions (PostGIS-Compatible)

RaisinDB supports GeoJSON geometries as native property values with PostGIS-compatible SQL functions:

```sql
-- Create a point geometry
SELECT ST_POINT(-122.4194, 37.7749) AS location;

-- Parse GeoJSON text to geometry
SELECT ST_GEOMFROMGEOJSON('{"type":"Point","coordinates":[-122.4194,37.7749]}');

-- Convert geometry back to GeoJSON text
SELECT ST_ASGEOJSON(properties -> 'location') FROM default WHERE path = '/places/sf';

-- Calculate distance between two points (meters, Haversine)
SELECT ST_DISTANCE(ST_POINT(-122.4194, 37.7749), ST_POINT(-73.9857, 40.7484));

-- Find places within 1000 meters of a point
SELECT * FROM default
WHERE ST_DWITHIN(properties -> 'location', ST_POINT(-122.4194, 37.7749), 1000);

-- Spatial predicates
SELECT * FROM default WHERE ST_CONTAINS(properties -> 'boundary', ST_POINT(-122.4, 37.7));
SELECT * FROM default WHERE ST_WITHIN(properties -> 'location', properties -> 'service_area');
SELECT * FROM default WHERE ST_INTERSECTS(properties -> 'route', properties -> 'zone');

-- Extract coordinates from a point
SELECT ST_X(properties -> 'location') AS longitude,
       ST_Y(properties -> 'location') AS latitude
FROM default WHERE node_type = 'geo:Place';
```

**Available ST_* functions**:
| Function | Returns | Description |
|----------|---------|-------------|
| `ST_POINT(lon, lat)` | GEOMETRY | Create point from WGS84 coordinates |
| `ST_GEOMFROMGEOJSON(text)` | GEOMETRY | Parse GeoJSON string |
| `ST_ASGEOJSON(geom)` | TEXT | Geometry to GeoJSON string |
| `ST_DISTANCE(a, b)` | DOUBLE | Distance in meters (Haversine) |
| `ST_DWITHIN(a, b, meters)` | BOOLEAN | Within distance check |
| `ST_CONTAINS(a, b)` | BOOLEAN | A contains B |
| `ST_WITHIN(a, b)` | BOOLEAN | A within B |
| `ST_INTERSECTS(a, b)` | BOOLEAN | Geometries intersect |
| `ST_X(point)` | DOUBLE | Get longitude |
| `ST_Y(point)` | DOUBLE | Get latitude |

**Supported GeoJSON types**: Point, LineString, Polygon, MultiPoint, MultiLineString, MultiPolygon, GeometryCollection

**Storage**: Geometries stored as `PropertyValue::Geometry(GeoJson)` with geohash-based spatial indexing.

---

## RaisinDB SQL Reference

### Insert a Node

```sql
-- Required columns: path, node_type
-- Optional: id (auto-generated if not provided), properties
INSERT INTO default (path, node_type, name)
VALUES ('/content/blog/my-post', 'raisin:Page', 'My Blog Post');

-- Insert with properties as JSON
INSERT INTO default (path, node_type, name, properties)
VALUES (
  '/products/laptop',
  'shop:Product',
  'Gaming Laptop',
  '{"price": 999.99, "stock": 50, "category": "electronics"}'
);
```

### Insert with JSON Properties

```sql
-- Insert with flat JSON properties
INSERT INTO default (path, node_type, name, properties)
VALUES (
  '/products/laptop',
  'shop:Product',
  'Gaming Laptop',
  '{"price": 999.99, "stock": 50, "featured": true}'
);

-- Insert with nested JSON objects
INSERT INTO default (path, node_type, name, properties)
VALUES (
  '/content/blog/post1',
  'raisin:Page',
  'My First Post',
  '{
    "title": "Welcome to My Blog",
    "status": "published",
    "author": "john@example.com",
    "seo": {
      "title": "Welcome | My Blog",
      "description": "Introduction post",
      "keywords": ["blog", "welcome"]
    },
    "metadata": {
      "views": 0,
      "likes": 0
    }
  }'
);

-- Insert with arrays in properties
INSERT INTO default (path, node_type, name, properties)
VALUES (
  '/content/articles/tech',
  'cms:Article',
  'Tech Article',
  '{
    "tags": ["rust", "database", "performance"],
    "categories": ["technology", "programming"],
    "relatedIds": ["id1", "id2", "id3"]
  }'
);
```

### Conditional JSON Updates

```sql
-- Update only if property exists
UPDATE default
SET properties = properties || '{"views": 100}'
WHERE path = '/content/blog/post1'
  AND properties ? 'views';

-- Update based on JSON value
UPDATE default
SET properties = properties || '{"status": "featured"}'
WHERE properties ->> 'status' = 'published'
  AND (properties ->> 'views')::int > 1000;

-- Update where nested property matches
UPDATE default
SET properties = jsonb_set(properties, '{seo,indexed}', 'true')
WHERE properties -> 'seo' ->> 'title' IS NOT NULL
  AND node_type = 'raisin:Page';

-- Bulk update with JSON conditions
UPDATE default
SET properties = properties || '{"needsReview": true}'
WHERE properties ->> 'status' = 'draft'
  AND (properties ->> 'createdAt')::timestamp < '2024-01-01';
```

### Auto-Commit Mode

DML without BEGIN auto-commits each statement:

```sql
-- Without BEGIN, each statement auto-commits immediately
-- This creates a separate revision for each operation

-- Auto-commits immediately
INSERT INTO default (path, node_type, name)
VALUES ('/content/page1', 'raisin:Page', 'Page 1');

-- This is a separate commit
INSERT INTO default (path, node_type, name)
VALUES ('/content/page2', 'raisin:Page', 'Page 2');

-- Use BEGIN/COMMIT to batch operations into single revision
```

### Create Folder Structure

```sql
-- Create a folder structure atomically
BEGIN;

-- Create parent folder first
INSERT INTO default (path, node_type, name)
VALUES ('/content/blog', 'raisin:Folder', 'Blog');

-- Then create child folders
INSERT INTO default (path, node_type, name)
VALUES
  ('/content/blog/2024', 'raisin:Folder', '2024'),
  ('/content/blog/2024/january', 'raisin:Folder', 'January'),
  ('/content/blog/2024/february', 'raisin:Folder', 'February');

COMMIT WITH MESSAGE 'Created blog folder structure';
```

### Bulk Content Migration

```sql
-- Migrate content to new structure
BEGIN;

-- Create new category folders
INSERT INTO default (path, node_type, name)
VALUES
  ('/content/articles', 'raisin:Folder', 'Articles'),
  ('/content/articles/tech', 'raisin:Folder', 'Technology'),
  ('/content/articles/news', 'raisin:Folder', 'News');

COMMIT WITH MESSAGE 'Set up new content structure' ACTOR 'migration-script';
```

### COPY - Duplicate Single Node

```sql
-- COPY statement duplicates a node to a new location
-- Syntax: COPY <workspace> [IN BRANCH 'name'] SET path='<source>' TO path='<new-parent>' [AS 'new-name']
-- Note: Creates new node ID, only copies the single node (not descendants)

-- Copy a template page to a new location
COPY default SET path='/templates/blog-post' TO path='/content/blog';
-- Result: Creates /content/blog/blog-post with new ID

-- Copy with a new name using AS clause
COPY default SET path='/templates/product' TO path='/products' AS 'new-product';
-- Result: Creates /products/new-product with new ID

-- Copy by node ID
COPY default SET id='abc123' TO path='/content/archive';

-- Copy on a specific branch
COPY default IN BRANCH 'feature' SET path='/content/draft' TO path='/content/ready';

-- Note: COPY creates new IDs for all copied nodes
-- For preserving IDs, use MOVE instead
```

### COPY TREE - Duplicate Entire Subtree

```sql
-- COPY TREE statement duplicates a node AND all descendants
-- Syntax: COPY TREE <workspace> [IN BRANCH 'name'] SET path='<source>' TO path='<new-parent>' [AS 'new-name']
-- All copied nodes get new IDs

-- Copy an entire folder with all contents
COPY TREE default SET path='/templates/site-section' TO path='/content';
-- Result: Creates /content/site-section/ with all descendants

-- Copy subtree with a new root name
COPY TREE default SET path='/archive/2023' TO path='/reference' AS '2023-archive';
-- Result: Creates /reference/2023-archive/ with all descendants

-- Copy by node ID
COPY TREE default SET id='abc123' TO path='/backup';

-- Copy tree on a specific branch
COPY TREE default IN BRANCH 'staging' SET path='/content/blog' TO path='/content/blog-backup';

-- Use case: Create content from templates
COPY TREE default SET path='/templates/landing-page' TO path='/campaigns' AS 'summer-sale';

-- Note: For large trees (>5000 nodes), operation may run as background job
```

### COPY vs MOVE Comparison

```sql
-- MOVE: Reparent nodes (keeps IDs, changes paths)
-- External references remain valid
-- History and audit trails preserved
-- Best for reorganizing content
MOVE default SET path='/blog/draft-post' TO path='/blog/published';
-- Node ID stays the same, path changes

-- COPY: Duplicate nodes (new IDs, new paths)
-- Creates independent copies
-- Original content unaffected
-- Best for templates and content duplication
COPY default SET path='/templates/post' TO path='/blog' AS 'new-post';
-- New node with new ID is created

-- COPY TREE: Duplicate entire subtrees
-- Copies node AND all descendants
-- All copied nodes get new IDs
-- Best for duplicating entire sections
COPY TREE default SET path='/templates/section' TO path='/content';
-- Creates copy of entire subtree with new IDs
```

### Create Basic NodeType

```sql
-- Create a basic Page node type with UI hints
-- Use LABEL for display name, DESCRIPTION for help text
-- Use ORDER for field ordering in forms
CREATE NODETYPE 'raisin:Page' (
  PROPERTIES (
    title String REQUIRED LABEL 'Page Title' ORDER 1,
    body String LABEL 'Content' DESCRIPTION 'Main page content' ORDER 2,
    author String LABEL 'Author' ORDER 3,
    status String DEFAULT 'draft' LABEL 'Status' ORDER 4,
    views Number DEFAULT 0 LABEL 'View Count',
    featured Boolean DEFAULT false LABEL 'Featured'
  )
);
```

### Create NodeType with Nested Object

```sql
-- Create a node type with deeply nested SEO object
CREATE NODETYPE 'cms:Article' (
  PROPERTIES (
    title String REQUIRED LABEL 'Title' ORDER 1,
    slug String REQUIRED UNIQUE LABEL 'URL Slug' ORDER 2,
    content String FULLTEXT LABEL 'Content' ORDER 3,
    seo Object {
      basic Object {
        title String LABEL 'SEO Title',
        description String TRANSLATABLE LABEL 'Meta Description'
      },
      social Object {
        og_title String LABEL 'Open Graph Title',
        og_image Resource LABEL 'OG Image',
        twitter_card String DEFAULT 'summary_large_image'
      }
    } LABEL 'SEO Settings' ORDER 4,
    published_at Date LABEL 'Publish Date'
  )
);
```

### Create NodeType with Indexes

```sql
-- Create a searchable Product node type
-- Use FULLTEXT modifier for full-text search
-- Use PROPERTY_INDEX for fast filtering
CREATE NODETYPE 'shop:Product' (
  PROPERTIES (
    name String REQUIRED FULLTEXT,
    description String FULLTEXT,
    sku String REQUIRED UNIQUE PROPERTY_INDEX,
    price Number REQUIRED PROPERTY_INDEX,
    category String PROPERTY_INDEX,
    tags Array OF String FULLTEXT,
    in_stock Boolean DEFAULT true
  )
);

-- FULLTEXT enables FULLTEXT_MATCH() searches
-- PROPERTY_INDEX speeds up WHERE clause filtering
```

### Create NodeType that Extends Another

```sql
-- Create a base Content type
CREATE NODETYPE 'cms:Content' (
  PROPERTIES (
    title String REQUIRED FULLTEXT,
    slug String,
    status String DEFAULT 'draft',
    author String
  )
  VERSIONABLE
  PUBLISHABLE
);

-- Create Article that extends Content
CREATE NODETYPE 'cms:Article' (
  EXTENDS 'cms:Content'
  PROPERTIES (
    body String FULLTEXT TRANSLATABLE,
    excerpt String,
    featured_image Resource,
    category String
  )
);

-- Article inherits all Content properties plus its own
```

### Create NodeType with Allowed Children

```sql
-- Create a Blog that can only contain Posts and Categories
CREATE NODETYPE 'cms:Blog' (
  PROPERTIES (
    name String REQUIRED,
    description String
  )
  ALLOWED_CHILDREN ('cms:Post', 'cms:Category')
);

-- Create a Category that can contain Posts
CREATE NODETYPE 'cms:Category' (
  PROPERTIES (
    name String REQUIRED,
    slug String UNIQUE
  )
  ALLOWED_CHILDREN ('cms:Post')
);

-- Create Post (leaf node - no children specified)
CREATE NODETYPE 'cms:Post' (
  PROPERTIES (
    title String REQUIRED FULLTEXT,
    body String FULLTEXT
  )
  PUBLISHABLE
);
```

### Create NodeType with Flags

```sql
-- All available flags for node types (flags go INSIDE the parentheses)
CREATE NODETYPE 'cms:Document' (
  DESCRIPTION 'Versioned document type'
  ICON 'document'
  PROPERTIES (
    title String REQUIRED,
    content String FULLTEXT
  )
  VERSIONABLE   -- Enable version history
  PUBLISHABLE   -- Enable publish workflow
  AUDITABLE     -- Track all changes
  INDEXABLE     -- Include in search indexes (default: true)
  STRICT        -- Enforce strict property validation
);
```

### Create NodeType with Compound Index

```sql
-- Compound indexes optimize ORDER BY + filter queries
-- Columns order: equality columns first, then ordering column last with ASC/DESC
CREATE NODETYPE 'news:Article' (
  PROPERTIES (
    title String REQUIRED FULLTEXT LABEL 'Title' ORDER 1,
    slug String REQUIRED PROPERTY_INDEX LABEL 'URL Slug' ORDER 2,
    category String PROPERTY_INDEX LABEL 'Category' ORDER 3,
    status String DEFAULT 'draft' PROPERTY_INDEX LABEL 'Status' ORDER 4,
    featured Boolean DEFAULT false PROPERTY_INDEX LABEL 'Featured' ORDER 5
  )
  COMPOUND_INDEX 'idx_article_category_status_created' ON (
    __node_type,
    category,
    status,
    __created_at DESC
  )
  PUBLISHABLE
  INDEXABLE
);

-- System fields available in compound indexes:
-- __node_type   - the node type (String)
-- __created_at  - creation timestamp (Timestamp)
-- __updated_at  - last update timestamp (Timestamp)
```

### DROP NODETYPE

```sql
-- Drop a node type (will fail if nodes of this type exist)
DROP NODETYPE 'myapp:OldType';

-- Drop with CASCADE to also delete all nodes of this type
DROP NODETYPE 'myapp:OldType' CASCADE;

-- NOTE: IF EXISTS is NOT supported
-- This will error if the type doesn't exist:
DROP NODETYPE 'myapp:NonExistent';  -- Error!

-- Typical pattern for setup scripts:
DROP NODETYPE 'news:Article';  -- May error on first run
DROP NODETYPE 'news:Tag';
CREATE NODETYPE 'news:Tag' (...);
CREATE NODETYPE 'news:Article' (...);
```

### All Property Types

```sql
-- All supported property types in RaisinDB DDL
CREATE NODETYPE 'demo:AllTypes' (
  PROPERTIES (
    -- Basic types
    text_field String LABEL 'Text',
    number_field Number LABEL 'Number',
    bool_field Boolean LABEL 'Boolean',
    date_field Date LABEL 'Date/Time',
    url_field URL LABEL 'URL',

    -- Reference types
    node_ref Reference LABEL 'Node Reference',
    type_ref NodeType LABEL 'NodeType Reference',

    -- Media/File
    file_field Resource LABEL 'File/Media',

    -- Rich content
    composite_field Composite LABEL 'Rich Content Blocks',
    element_field Element LABEL 'Single Element',

    -- Array types
    string_list Array OF String LABEL 'String List',
    number_list Array OF Number LABEL 'Number List',
    ref_list Array OF Reference LABEL 'Reference List',

    -- Object types with nesting
    metadata Object {
      created_by String,
      tags Array OF String
    } LABEL 'Metadata',

    -- Object with ALLOW_ADDITIONAL_PROPERTIES
    custom_data Object {
      known_field String
    } ALLOW_ADDITIONAL_PROPERTIES LABEL 'Custom Data'
  )
);
```

### Complex E-commerce Product

```sql
-- Complex product type with deep nesting
CREATE NODETYPE 'ecommerce:Product' (
  EXTENDS 'raisin:Node'
  DESCRIPTION 'E-commerce product'
  ICON 'shopping-cart'
  PROPERTIES (
    name String REQUIRED FULLTEXT LABEL 'Product Name' ORDER 1,
    sku String REQUIRED UNIQUE PROPERTY_INDEX LABEL 'SKU' ORDER 2,
    price Number REQUIRED PROPERTY_INDEX LABEL 'Price' ORDER 3,

    media Object {
      primary_image Resource REQUIRED LABEL 'Main Image',
      gallery Array OF Resource LABEL 'Gallery',
      videos Array OF Object {
        url URL REQUIRED,
        title String,
        thumbnail Resource
      }
    } LABEL 'Media' ORDER 4,

    specs Object {
      dimensions Object {
        width Number LABEL 'Width (cm)',
        height Number LABEL 'Height (cm)',
        weight Number LABEL 'Weight (kg)'
      },
      custom Object {} ALLOW_ADDITIONAL_PROPERTIES
    } LABEL 'Specifications' ORDER 5,

    seo Object {
      title String LABEL 'SEO Title',
      description String TRANSLATABLE,
      keywords Array OF String
    } LABEL 'SEO' ORDER 6
  )
  ALLOWED_CHILDREN ('ecommerce:Variant')
  VERSIONABLE
  PUBLISHABLE
);
```

### Create Archetype

```sql
-- Create a Blog Post archetype based on cms:Article
CREATE ARCHETYPE 'blog-post'
BASE_NODE_TYPE 'cms:Article'
TITLE 'Blog Post'
DESCRIPTION 'Template for blog posts'
FIELDS (
  title String REQUIRED,
  body String FULLTEXT
)
PUBLISHABLE;

-- Archetypes provide default property values
-- when creating new nodes of that type
```

### Create ElementType

```sql
-- Create a Hero Banner element type
CREATE ELEMENTTYPE 'ui:HeroBanner'
DESCRIPTION 'Hero section with background image'
ICON 'image'
FIELDS (
  heading String REQUIRED,
  subheading String,
  background_image Resource,
  cta_text String,
  cta_link String,
  alignment String DEFAULT 'center'
);

-- Create a Card element type
CREATE ELEMENTTYPE 'ui:Card'
FIELDS (
  title String REQUIRED,
  description String,
  image Resource,
  link String
);

-- ElementTypes define reusable content blocks
```

### View Schema Tables

```sql
-- View all node types
SELECT id, name, properties, allowed_children
FROM NodeTypes
ORDER BY name;

-- View all archetypes
SELECT id, name, node_type_id, default_properties
FROM Archetypes
ORDER BY name;

-- View all element types
SELECT id, name, properties
FROM ElementTypes
ORDER BY name;

-- Note: NodeTypes, Archetypes, ElementTypes are read-only
-- Use DDL (CREATE/ALTER/DROP) to modify schema
```
