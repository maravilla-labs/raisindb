-- Full-Text Search in RaisinDB (PostgreSQL-Compatible)
-- Based on PostgreSQL's tsvector, tsquery, and ranking approach

-- ============================================================================
-- BASIC FULL-TEXT SEARCH CONCEPTS
-- ============================================================================

-- PostgreSQL uses:
-- - tsvector: tokenized, normalized representation of text (indexed document)
-- - tsquery: search query with boolean logic
-- - @@: "matches" operator
-- - ts_rank(): relevance scoring

-- ============================================================================
-- 1. GENERATE TSVECTOR FROM TEXT
-- ============================================================================

-- Convert text to tsvector using English configuration
-- This tokenizes, removes stop words, and stems words
SELECT
    id,
    name,
    to_tsvector('english', properties ->> 'body') AS document
FROM nodes
WHERE node_type = 'my:Article';

-- Multiple fields into one tsvector
SELECT
    id,
    to_tsvector('english',
        coalesce(properties ->> 'title', '') || ' ' ||
        coalesce(properties ->> 'body', '')
    ) AS document
FROM nodes;

-- ============================================================================
-- 2. BASIC SEARCH WITH @@ OPERATOR
-- ============================================================================

-- Simple word search
SELECT id, name, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'rust');

-- Multiple words (AND)
SELECT id, name, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'rust & performance');

-- Multiple words (OR)
SELECT id, name, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'rust | python');

-- NOT operator
SELECT id, name, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'rust & !python');

-- Complex boolean query
SELECT id, name, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', '(rust | python) & performance');

-- ============================================================================
-- 3. RANKING SEARCH RESULTS
-- ============================================================================

-- Rank results by relevance using ts_rank()
SELECT
    id,
    name,
    properties ->> 'title' AS title,
    ts_rank(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'rust & code')
    ) AS rank
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'rust & code')
ORDER BY rank DESC;

-- Rank with coverage density (ts_rank_cd)
-- More precise ranking considering term proximity
SELECT
    id,
    properties ->> 'title' AS title,
    ts_rank_cd(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'optimize & performance')
    ) AS rank
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'optimize & performance')
ORDER BY rank DESC
LIMIT 10;

-- ============================================================================
-- 4. MULTI-FIELD SEARCH WITH WEIGHTS
-- ============================================================================

-- Search across title and body with different weights
-- A = highest weight, D = lowest weight
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

-- Three fields: title (A), description (B), body (C)
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

-- ============================================================================
-- 5. PREFIX SEARCH (AUTOCOMPLETE)
-- ============================================================================

-- Prefix search using :* operator
-- Matches: perform, performance, performing, etc.
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'perform:*');

-- Multiple prefix terms
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'rust:* & optim:*');

-- ============================================================================
-- 6. PHRASE SEARCH (PROXIMITY)
-- ============================================================================

-- Words within 2 positions of each other
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'rust <2> performance');

-- Exact adjacency (within 1 word)
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ to_tsquery('english', 'database <1> system');

-- ============================================================================
-- 7. SIMPLE QUERY PARSING (USER-FRIENDLY)
-- ============================================================================

-- plainto_tsquery() - converts plain text to tsquery
-- Automatically adds & between words
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ plainto_tsquery('english', 'rust programming language');

-- Equivalent to: 'rust & programming & language'

-- phraseto_tsquery() - treats input as a phrase
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ phraseto_tsquery('english', 'high performance computing');

-- websearch_to_tsquery() - Google-like search syntax
-- Supports quotes, OR, -, etc.
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
    @@ websearch_to_tsquery('english', 'rust OR python -javascript');

-- ============================================================================
-- 8. LANGUAGE-SPECIFIC SEARCH
-- ============================================================================

-- English configuration
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'running');
-- Matches: run, running, runs

-- German configuration
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('german', properties ->> 'body') @@ to_tsquery('german', 'laufen');
-- Uses German stemming rules

-- Simple configuration (no stemming, no stop words)
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector('simple', properties ->> 'body') @@ to_tsquery('simple', 'running');
-- Only matches exact word "running"

-- Dynamic language selection based on node property
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE to_tsvector(
    coalesce(properties ->> 'language', 'english'),
    properties ->> 'body'
) @@ to_tsquery(
    coalesce(properties ->> 'language', 'english'),
    'search & terms'
);

-- ============================================================================
-- 9. COMBINED WITH RAISINDB FEATURES
-- ============================================================================

-- Full-text search within specific path
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
AND to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'rust')
ORDER BY rank DESC;

-- Full-text search on direct children
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
AND to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'database')
ORDER BY rank DESC;

-- Full-text with property filters
SELECT
    id,
    properties ->> 'title' AS title,
    properties ->> 'status' AS status,
    ts_rank(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'performance')
    ) AS rank
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'performance')
AND properties ->> 'status' = 'published'
AND JSON_EXISTS(properties, '$.featured')
ORDER BY rank DESC;

-- ============================================================================
-- 10. DEBUGGING FULL-TEXT SEARCH
-- ============================================================================

-- See how text is tokenized
SELECT to_tsvector('english', 'Running faster with Rust programming language');
-- Output: 'fast':2 'languag':6 'program':5 'run':1 'rust':4

-- See how query is parsed
SELECT to_tsquery('english', 'rust & performance');
-- Output: 'rust' & 'perform'

-- Use plainto_tsquery for debugging
SELECT plainto_tsquery('english', 'rust performance optimization');
-- Output: 'rust' & 'perform' & 'optim'

-- Check if query matches document
SELECT
    'rust programming language'::text AS original,
    to_tsvector('english', 'rust programming language') AS vector,
    to_tsquery('english', 'rust & program') AS query,
    to_tsvector('english', 'rust programming language') @@ to_tsquery('english', 'rust & program') AS matches;

-- ============================================================================
-- 11. PAGINATION WITH FULL-TEXT SEARCH
-- ============================================================================

-- Cursor-based pagination with full-text search
SELECT
    id,
    properties ->> 'title' AS title,
    created_at,
    ts_rank(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'rust')
    ) AS rank
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'rust')
ORDER BY rank DESC, created_at DESC, id DESC
LIMIT 20;

-- Next page (using last item's rank and created_at as cursor)
SELECT
    id,
    properties ->> 'title' AS title,
    created_at,
    ts_rank(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'rust')
    ) AS rank
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'rust')
AND (
    ts_rank(to_tsvector('english', properties ->> 'body'), to_tsquery('english', 'rust')) < 0.5
    OR (
        ts_rank(to_tsvector('english', properties ->> 'body'), to_tsquery('english', 'rust')) = 0.5
        AND created_at < '2025-01-15T10:00:00Z'
    )
)
ORDER BY rank DESC, created_at DESC, id DESC
LIMIT 20;

-- ============================================================================
-- 12. AGGREGATIONS WITH FULL-TEXT SEARCH
-- ============================================================================

-- Count matches by category
SELECT
    properties ->> 'category' AS category,
    COUNT(*) AS match_count
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'rust')
GROUP BY properties ->> 'category'
ORDER BY match_count DESC;

-- Average rank by node type
SELECT
    node_type,
    AVG(ts_rank(
        to_tsvector('english', properties ->> 'body'),
        to_tsquery('english', 'database')
    )) AS avg_rank,
    COUNT(*) AS count
FROM nodes
WHERE to_tsvector('english', properties ->> 'body') @@ to_tsquery('english', 'database')
GROUP BY node_type;

-- ============================================================================
-- PERFORMANCE NOTES
-- ============================================================================

-- For optimal performance, RaisinDB should support:
-- 1. Generated column for tsvector:
--    ALTER TABLE nodes ADD COLUMN document tsvector
--    GENERATED ALWAYS AS (
--      setweight(to_tsvector('english', properties ->> 'title'), 'A') ||
--      setweight(to_tsvector('english', properties ->> 'body'), 'B')
--    ) STORED;
--
-- 2. GIN index on tsvector:
--    CREATE INDEX idx_nodes_fts ON nodes USING GIN (document);
--
-- 3. Then queries become:
--    SELECT * FROM nodes
--    WHERE document @@ to_tsquery('english', 'rust & performance')
--    ORDER BY ts_rank(document, to_tsquery('english', 'rust & performance')) DESC;

-- ============================================================================
-- RAISINDB IMPLEMENTATION NOTES
-- ============================================================================

-- In RaisinDB schema, this maps to:
-- {
--   "name": "body",
--   "type": "String",
--   "index": {
--     "kind": "fulltext",
--     "language": "en",  // maps to PostgreSQL 'english' config
--     "fields": ["title:A", "body:B"]  // weighted fields
--   }
-- }

-- At query time, RaisinDB can:
-- 1. Parse tsquery using same PostgreSQL logic
-- 2. Use Tantivy for the actual full-text index
-- 3. Return results with PostgreSQL-compatible ranking
-- 4. Support same operators: &, |, !, :*, <N>

-- This gives PostgreSQL API with Tantivy performance!
