import { useState } from 'react'
import { X, Search, ChevronDown, ChevronRight, Code2, Database, Network, GitBranch, Sparkles, BookOpen, Zap, Globe, PenLine, Clock, Shield } from 'lucide-react'

interface SqlHelpSidebarProps {
  isOpen: boolean
  onClose: () => void
  onInsertExample: (sql: string) => void
  repo: string
}

interface Example {
  title: string
  description: string
  sql: string
}

interface Section {
  id: string
  title: string
  icon: React.ReactNode
  description: string
  examples: Example[]
}

export function SqlHelpSidebar({ isOpen, onClose, onInsertExample, repo }: SqlHelpSidebarProps) {
  const [searchTerm, setSearchTerm] = useState('')
  const [expandedSections, setExpandedSections] = useState<Record<string, boolean>>({
    hierarchy: true, // Start with hierarchy expanded as it's priority #1
  })

  const toggleSection = (sectionId: string) => {
    setExpandedSections(prev => ({ ...prev, [sectionId]: !prev[sectionId] }))
  }

  const handleInsert = (sql: string) => {
    onInsertExample(sql)
    // Don't close sidebar - user might want to reference multiple examples
  }

  const sections: Section[] = [
    {
      id: 'hierarchy',
      title: 'Hierarchy & Path Queries',
      icon: <Network className="w-4 h-4" />,
      description: 'Query hierarchical node structures, children, descendants, and paths',
      examples: [
        {
          title: 'Get Direct Children',
          description: 'Get immediate children of a parent node using PARENT() function',
          sql: `-- Get direct children of a node (most efficient)
SELECT id, name, path, node_type, properties
FROM default
WHERE PARENT(path) = '/content/blog'
ORDER BY name;`,
        },
        {
          title: 'Get All Descendants (Subtree)',
          description: 'Get entire subtree including all nested descendants',
          sql: `-- Get all descendants under a path
SELECT id, name, path, DEPTH(path) AS depth_level
FROM default
WHERE PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY path;`,
        },
        {
          title: 'DESCENDANT_OF Function',
          description:
            'Use DESCENDANT_OF for implicit path checking with optional depth limit',
          sql: `-- DESCENDANT_OF: Check if row is a descendant of a parent path
-- Simpler than PATH_STARTS_WITH (no explicit path column needed)

-- Get all descendants (any depth)
SELECT id, name, path, DEPTH(path) AS depth_level
FROM default
WHERE DESCENDANT_OF('/content/blog')
ORDER BY path;

-- Limit to specific depth (2 levels max)
SELECT id, name, path
FROM default
WHERE DESCENDANT_OF('/content/blog', 2)
ORDER BY path;

-- Direct children only (depth = 1, equivalent to using PARENT)
SELECT * FROM default WHERE DESCENDANT_OF('/content', 1);

-- Comparison:
-- PARENT(path) = '/content'       → direct children only
-- DESCENDANT_OF('/content')       → all descendants (unlimited)
-- DESCENDANT_OF('/content', 1)    → same as PARENT check
-- DESCENDANT_OF('/content', 2)    → children + grandchildren`,
        },
        {
          title: 'Limit Descendant Depth',
          description: 'Query descendants but limit how deep in the hierarchy',
          sql: `-- Get descendants up to 2 levels deep
SELECT
  id,
  name,
  path,
  DEPTH(path) - DEPTH('/content/blog/') AS relative_depth
FROM default
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND DEPTH(path) <= DEPTH('/content/blog/') + 2
ORDER BY path;`,
        },
        {
          title: 'Parent-Child JOIN',
          description: 'Join nodes with their parent nodes to get parent metadata',
          sql: `-- Get children with parent information
SELECT
  c.id AS child_id,
  c.name AS child_name,
  c.path AS child_path,
  p.id AS parent_id,
  p.name AS parent_name,
  p.properties AS parent_props
FROM default c
LEFT JOIN default p ON p.path = PARENT(c.path)
WHERE PARENT(c.path) = '/content'
ORDER BY c.name;`,
        },
        {
          title: 'Get Root-Level Nodes',
          description: 'Query only top-level nodes at depth 1',
          sql: `-- Get all root-level nodes
SELECT id, name, path
FROM default
WHERE DEPTH(path) = 1
ORDER BY name;`,
        },
        {
          title: 'Count Children',
          description: 'Count direct children of multiple parent nodes',
          sql: `-- Count children per parent
SELECT
  PARENT(path) AS parent_path,
  COUNT(*) AS child_count
FROM default
WHERE DEPTH(path) = 2
GROUP BY PARENT(path)
ORDER BY child_count DESC;`,
        },
        {
          title: 'Get Ancestor at Specific Depth',
          description: 'Get ancestor node at a specific depth level from root using ANCESTOR()',
          sql: `-- Get ancestor at depth 2 (e.g., category level)
SELECT
  path,
  ANCESTOR(path, 2) AS category_ancestor,
  ANCESTOR(path, 1) AS root_ancestor,
  DEPTH(path) AS current_depth
FROM default
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND DEPTH(path) >= 2
ORDER BY path;

-- Example results:
-- path: /content/blog/2024/post1 → category_ancestor: /content/blog
-- path: /content/blog/2024 → category_ancestor: /content/blog
-- path: /content/blog → category_ancestor: /content/blog`,
        },
        {
          title: 'Group by Ancestor Level',
          description: 'Group content by ancestor at specific depth for hierarchical aggregation',
          sql: `-- Group documents by their depth-2 ancestor (e.g., category)
SELECT
  ANCESTOR(path, 2) AS category,
  COUNT(*) AS doc_count,
  ARRAY_AGG(name) AS doc_names
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
  AND DEPTH(path) >= 3
GROUP BY ANCESTOR(path, 2)
ORDER BY doc_count DESC;`,
        },
        {
          title: 'Navigate Multiple Levels Up',
          description: 'Get grandparent or great-grandparent using PARENT with levels parameter',
          sql: `-- Get parent, grandparent, and great-grandparent
SELECT
  path,
  name,
  PARENT(path) AS parent,           -- 1 level up (default)
  PARENT(path, 1) AS parent_explicit, -- Same as above
  PARENT(path, 2) AS grandparent,     -- 2 levels up
  PARENT(path, 3) AS great_grandparent -- 3 levels up
FROM default
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND DEPTH(path) >= 4
ORDER BY path
LIMIT 10;`,
        },
        {
          title: 'Find Nodes with Same Grandparent',
          description: 'Find sibling branches using multi-level PARENT function',
          sql: `-- Find all nodes that share the same grandparent
-- Useful for finding related content in parallel branches
SELECT
  d1.path AS path1,
  d2.path AS path2,
  PARENT(d1.path, 2) AS shared_grandparent,
  d1.name AS name1,
  d2.name AS name2
FROM default d1
JOIN default d2 ON PARENT(d1.path, 2) = PARENT(d2.path, 2)
WHERE d1.path < d2.path  -- Avoid duplicates
  AND PATH_STARTS_WITH(d1.path, '/content/')
  AND DEPTH(d1.path) >= 3
LIMIT 20;`,
        },
        {
          title: 'Pattern Matching with Relative Parent',
          description: 'Find documents based on ancestor patterns using PARENT levels',
          sql: `-- Find all blog posts whose grandparent ends with '/blog'
-- Useful for finding deeply nested content by category
SELECT
  id,
  name,
  path,
  DEPTH(path) AS depth,
  PARENT(path, 2) AS grandparent
FROM default
WHERE PARENT(path, 2) LIKE '%/blog'
  AND DEPTH(path) >= 3
ORDER BY path;`,
        },
        {
          title: 'Reorder Children (ORDER Statement)',
          description: 'Change the order of sibling nodes using ORDER with ABOVE/BELOW',
          sql: `-- To reorder children within a parent, use the ORDER statement
-- See "Data Manipulation (DML)" section for full examples

-- Move a child before another sibling
ORDER default SET path='/content/blog/post3' ABOVE path='/content/blog/post1';

-- Move a child after another sibling
ORDER default SET path='/content/blog/post1' BELOW path='/content/blog/post2';

-- Both nodes must share the same parent path`,
        },
        {
          title: 'Move Node to New Parent (MOVE Statement)',
          description: 'Reparent a node subtree while preserving node IDs',
          sql: `-- To move a node (and descendants) to a new parent, use MOVE
-- See "Data Manipulation (DML)" section for full examples

-- Move a subtree to a different parent
MOVE default SET path='/content/blog/draft' TO path='/content/published';

-- The node keeps its ID and name, only path changes
-- Result: /content/blog/draft → /content/published/draft`,
        },
      ],
    },
    {
      id: 'graph',
      title: 'Graph & Cypher Queries',
      icon: <Network className="w-4 h-4" />,
      description: 'Query graph relationships using Cypher syntax and path-first traversals',
      examples: [
        {
          title: 'Basic Cypher Query',
          description: 'Match nodes and relationships using ASCII-art patterns',
          sql: `-- Find all articles that 'continue' from a specific article
CYPHER '
MATCH (prev)-[:continues]->(next)
WHERE prev.path = "/superbigshit/articles/tech/ai-revolution-part-1"
RETURN next.id, next.name, next.path
'`,
        },
        {
          title: 'Path-First Traversal',
          description: 'Optimize queries by anchoring to a specific path',
          sql: `-- Efficiently find neighbors of a specific node
CYPHER '
MATCH (this)-[:similar_to]->(other)
WHERE this.path = "/superbigshit/articles/tech/rust-web-development-2025"
RETURN other.name, other.path
ORDER BY other.name
'`,
        },
        {
          title: 'Filtering Relationships',
          description: 'Filter by relationship type or properties',
          sql: `-- Find specific types of relationships
CYPHER '
MATCH (this)-[r]->(other)
WHERE this.path = "/superbigshit/articles/tech/ai-coding-assistants"
  AND type(r) IN ["similar-to", "see-also"]
RETURN other.name, type(r) as relation, r.weight
ORDER BY r.weight DESC
'`,
        },
        {
          title: 'Incoming vs Outgoing',
          description: 'Control direction of traversal',
          sql: `-- Find incoming references (who points to this?)
CYPHER '
MATCH (source)-[:corrects]->(target)
WHERE target.path = "/superbigshit/articles/tech/ai-coding-assistants"
RETURN source.name, source.path
'`,
        },
        {
          title: 'Variable Length Paths',
          description: 'Traverse multiple hops',
          sql: `-- Find indirect connections (up to 2 hops)
CYPHER '
MATCH (start)-[:continues*1..2]->(end)
WHERE start.path = "/superbigshit/articles/tech/ai-revolution-part-1"
RETURN end.name, end.path
'`,
        },
      ],
    },
    {
      id: 'search',
      title: 'Full-Text & Vector Search',
      icon: <Search className="w-4 h-4" />,
      description: 'Search content with Tantivy full-text indexing and semantic vector similarity',
      examples: [
        {
          title: 'Basic Full-Text Search',
          description: 'Search indexed content using Tantivy engine',
          sql: `-- Full-text search across all indexed properties
-- Note: Only properties listed in node type's "properties_to_index" are searchable
SELECT
  id,
  name,
  path,
  node_type,
  properties ->> 'title' AS title,
  properties ->> 'body' AS body,
  properties ->> 'author' AS author,
  properties ->> 'published_date' AS published_date
FROM default
WHERE FULLTEXT_MATCH('rust AND performance', 'english')
ORDER BY updated_at DESC
LIMIT 20;`,
        },
        {
          title: 'Full-Text with Boolean Operators',
          description: 'Use Tantivy query syntax for complex searches',
          sql: `-- Boolean operators: AND, OR, NOT
-- Supports wildcards (*), fuzzy search (~), and more
SELECT
  id,
  name,
  path,
  node_type,
  properties ->> 'title' AS title,
  properties ->> 'status' AS status
FROM default
WHERE FULLTEXT_MATCH('(database OR storage) AND NOT legacy', 'english')
  AND properties ->> 'status' = 'published'
ORDER BY created_at DESC
LIMIT 50;`,
        },
        {
          title: 'Cross-Workspace Full-Text Search',
          description: 'Search across all workspaces using FULLTEXT_SEARCH table function',
          sql: `-- Search ALL workspaces at once (cross-workspace search)
-- Returns columns: node_id, workspace_id, name, path, node_type, score, revision, properties
SELECT
  node_id,
  workspace_id,
  name,
  path,
  node_type,
  score,
  properties ->> 'title' AS title,
  properties ->> 'body' AS body_preview
FROM FULLTEXT_SEARCH('rust AND performance', 'english')
ORDER BY score DESC
LIMIT 20;

-- Or select all columns:
-- SELECT * FROM FULLTEXT_SEARCH('query', 'en')

-- Note: FULLTEXT_SEARCH automatically searches ALL workspaces
-- Use FULLTEXT_MATCH in WHERE clause for single-workspace search`,
        },
        {
          title: 'Vector Similarity Search',
          description: 'Find similar nodes using vector embeddings and KNN',
          sql: `-- Find 10 most similar nodes using vector search
SELECT
  n.id,
  n.name,
  n.path,
  knn.distance,
  n.properties ->> 'title' AS title
FROM KNN(EMBEDDING('your search text here'), 10) AS knn
JOIN default n ON n.id = knn.node_id
ORDER BY knn.distance
LIMIT 10;`,
        },
        {
          title: 'Full-Text with Hierarchy Filter',
          description: 'Combine full-text search with path filtering',
          sql: `-- Search within a specific subtree
SELECT
  id,
  name,
  path,
  node_type,
  DEPTH(path) AS depth,
  properties ->> 'title' AS title
FROM default
WHERE FULLTEXT_MATCH('database performance optimization', 'english')
  AND PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY updated_at DESC
LIMIT 20;`,
        },
        {
          title: 'Multi-Language Full-Text',
          description: 'Search with language-specific stemming',
          sql: `-- German language search with proper stemming
SELECT
  id,
  name,
  path,
  properties ->> 'title' AS title,
  properties ->> 'description' AS description
FROM default
WHERE FULLTEXT_MATCH('datenbank AND leistung', 'german')
LIMIT 10;

-- English search
-- WHERE FULLTEXT_MATCH('database AND performance', 'english')`,
        },
        {
          title: 'Point-in-Time Full-Text Search',
          description: 'Search documents as they existed at a specific revision',
          sql: `-- Search at specific revision (via API context)
-- Set max_revision parameter when calling SQL API:
-- POST /api/repository/\${repo}/sql?max_revision=12345
-- Body: { "query": "SELECT ... WHERE FULLTEXT_MATCH(...)" }

SELECT
  id,
  name,
  path,
  version,
  properties ->> 'title' AS title
FROM default
WHERE FULLTEXT_MATCH('architecture', 'english')
ORDER BY updated_at DESC
LIMIT 20;

-- This returns documents indexed at or before revision 12345
-- Default (no max_revision): Returns latest/HEAD documents`,
        },
        {
          title: 'Tantivy Query Syntax Guide',
          description: 'Advanced search patterns supported by Tantivy',
          sql: `-- Tantivy supports powerful query syntax:
--
-- Boolean: AND, OR, NOT
--   'rust AND web'
--   '(rust OR python) AND NOT javascript'
--
-- Wildcards: * for prefix match
--   'perform*' matches 'performance', 'performer', etc.
--
-- Fuzzy search: ~ with edit distance
--   'performnce~2' matches 'performance'
--
-- Phrase search: "exact phrase"
--   '"high performance"'
--
-- Field-specific (if schema supports):
--   'title:rust body:async'

SELECT id, name, path, properties
FROM default
WHERE FULLTEXT_MATCH('rust* AND (web OR async~1)', 'english')
LIMIT 20;`,
        },
        {
          title: 'Vector Distance Operators',
          description: 'Use distance operators directly in queries',
          sql: `-- Compare vector embeddings using distance operators
SELECT
  id,
  name,
  path,
  node_type,
  properties ->> 'title' AS title,
  properties ->> 'description' AS description,
  properties -> 'embedding' <-> EMBEDDING('query text') AS l2_distance,
  properties -> 'embedding' <=> EMBEDDING('query text') AS cosine_distance,
  properties -> 'embedding' <#> EMBEDDING('query text') AS inner_product
FROM default
WHERE properties ? 'embedding'
  AND node_type = 'raisin:Page'
ORDER BY l2_distance
LIMIT 10;`,
        },
      ],
    },
    {
      id: 'json',
      title: 'JSON Functions',
      icon: <Code2 className="w-4 h-4" />,
      description: 'Extract and query JSON properties using JSONPath and type-safe accessors',
      examples: [
        {
          title: 'JSON_VALUE() - Extract with JSONPath',
          description: 'Extract scalar values from JSON using JSONPath expressions ($.path.to.field)',
          sql: `-- Extract nested JSON values using JSONPath
SELECT
  id,
  name,
  path,
  JSON_VALUE(properties, '$.title') AS title,
  JSON_VALUE(properties, '$.seo.title') AS seo_title,
  JSON_VALUE(properties, '$.metadata.category') AS category,
  JSON_VALUE(properties, '$.price') AS price_text
FROM default
WHERE JSON_VALUE(properties, '$.status') = 'published'
ORDER BY name
LIMIT 20;

-- JSONPath syntax:
-- $          - Root object
-- .field     - Object field access
-- .nested.field - Nested field access`,
        },
        {
          title: 'JSON_EXISTS() - Check Path Existence',
          description: 'Test if a JSONPath exists in the JSON document',
          sql: `-- Find nodes with specific JSON structure
SELECT
  id,
  name,
  path,
  properties ->> 'title' AS title
FROM default
WHERE JSON_EXISTS(properties, '$.seo.title')
  AND JSON_EXISTS(properties, '$.seo.description')
  AND node_type = 'raisin:Page'
ORDER BY path;

-- Use cases:
-- - Validate data completeness
-- - Find nodes missing required fields
-- - Filter by document structure`,
        },
        {
          title: 'Find Missing JSON Fields',
          description: 'Identify nodes that lack required JSON properties',
          sql: `-- Find published pages missing SEO metadata
SELECT
  id,
  name,
  path,
  properties ->> 'status' AS status,
  CASE
    WHEN NOT JSON_EXISTS(properties, '$.seo') THEN 'Missing SEO object'
    WHEN NOT JSON_EXISTS(properties, '$.seo.title') THEN 'Missing SEO title'
    WHEN NOT JSON_EXISTS(properties, '$.seo.description') THEN 'Missing SEO description'
    ELSE 'Complete'
  END AS seo_status
FROM default
WHERE properties ->> 'status' = 'published'
  AND node_type = 'raisin:Page'
  AND NOT JSON_EXISTS(properties, '$.seo.title')
ORDER BY path;`,
        },
        {
          title: 'JSON_GET_TEXT() - Simple Key Extraction',
          description: 'Extract top-level JSON string values with type conversion',
          sql: `-- Extract text fields from JSON properties
SELECT
  id,
  name,
  path,
  JSON_GET_TEXT(properties, 'title') AS title,
  JSON_GET_TEXT(properties, 'author') AS author,
  JSON_GET_TEXT(properties, 'status') AS status,
  JSON_GET_TEXT(properties, 'description') AS description
FROM default
WHERE node_type = 'raisin:Page'
  AND JSON_GET_TEXT(properties, 'status') = 'published'
ORDER BY JSON_GET_TEXT(properties, 'title')
LIMIT 20;`,
        },
        {
          title: 'JSON_GET_DOUBLE() - Numeric Extraction',
          description: 'Extract and filter by numeric JSON properties',
          sql: `-- Query products by price range
SELECT
  id,
  name,
  path,
  JSON_GET_TEXT(properties, 'title') AS title,
  JSON_GET_DOUBLE(properties, 'price') AS price,
  JSON_GET_DOUBLE(properties, 'rating') AS rating,
  JSON_GET_INT(properties, 'stock') AS stock_qty
FROM default
WHERE node_type = 'shop:Product'
  AND JSON_GET_DOUBLE(properties, 'price') > 10.0
  AND JSON_GET_DOUBLE(properties, 'price') <= 100.0
  AND JSON_GET_DOUBLE(properties, 'rating') >= 4.0
ORDER BY price DESC
LIMIT 20;`,
        },
        {
          title: 'JSON_GET_INT() - Integer Values',
          description: 'Extract integer properties for counting and aggregation',
          sql: `-- Analyze content by view counts
SELECT
  id,
  name,
  path,
  JSON_GET_TEXT(properties, 'title') AS title,
  JSON_GET_INT(properties, 'views') AS views,
  JSON_GET_INT(properties, 'likes') AS likes,
  JSON_GET_INT(properties, 'comments') AS comments
FROM default
WHERE node_type = 'raisin:Page'
  AND JSON_GET_INT(properties, 'views') > 1000
ORDER BY JSON_GET_INT(properties, 'views') DESC
LIMIT 20;`,
        },
        {
          title: 'JSON_GET_BOOL() - Boolean Flags',
          description: 'Filter by boolean JSON properties',
          sql: `-- Find featured and published content
SELECT
  id,
  name,
  path,
  JSON_GET_TEXT(properties, 'title') AS title,
  JSON_GET_BOOL(properties, 'featured') AS is_featured,
  JSON_GET_BOOL(properties, 'archived') AS is_archived,
  JSON_GET_TEXT(properties, 'category') AS category
FROM default
WHERE JSON_GET_BOOL(properties, 'featured') = true
  AND JSON_GET_BOOL(properties, 'archived') = false
  AND node_type = 'raisin:Page'
ORDER BY JSON_GET_TEXT(properties, 'published_date') DESC
LIMIT 20;`,
        },
        {
          title: 'Combine JSON Functions with Aggregation',
          description: 'Use JSON functions in GROUP BY and aggregate queries',
          sql: `-- Aggregate statistics by JSON category
SELECT
  JSON_GET_TEXT(properties, 'category') AS category,
  COUNT(*) AS total_items,
  AVG(JSON_GET_DOUBLE(properties, 'price')) AS avg_price,
  MIN(JSON_GET_DOUBLE(properties, 'price')) AS min_price,
  MAX(JSON_GET_DOUBLE(properties, 'price')) AS max_price,
  SUM(JSON_GET_INT(properties, 'quantity')) AS total_quantity
FROM default
WHERE node_type = 'shop:Product'
  AND JSON_EXISTS(properties, '$.price')
  AND JSON_EXISTS(properties, '$.quantity')
GROUP BY JSON_GET_TEXT(properties, 'category')
ORDER BY avg_price DESC;`,
        },
        {
          title: 'JSON Operators vs JSON Functions',
          description: 'When to use ->> operator vs JSON_GET_* functions',
          sql: `-- Both approaches work for simple extraction:

-- Using ->> operator (PostgreSQL style)
SELECT
  properties ->> 'title' AS title,
  properties ->> 'author' AS author
FROM default;

-- Using JSON_GET_TEXT function (explicit typing)
SELECT
  JSON_GET_TEXT(properties, 'title') AS title,
  JSON_GET_TEXT(properties, 'author') AS author
FROM default;

-- Use JSON_VALUE() for nested paths:
SELECT
  JSON_VALUE(properties, '$.seo.title') AS seo_title,
  JSON_VALUE(properties, '$.metadata.tags[0]') AS first_tag
FROM default;

-- Use typed JSON_GET_* for type safety:
SELECT
  JSON_GET_DOUBLE(properties, 'price') AS price,
  JSON_GET_INT(properties, 'quantity') AS qty,
  JSON_GET_BOOL(properties, 'active') AS active
FROM default;`,
        },
        {
          title: 'Advanced JSONPath Patterns',
          description: 'Complex JSON querying with nested structures',
          sql: `-- Query deeply nested JSON structures
SELECT
  id,
  name,
  path,
  -- Top-level fields
  JSON_VALUE(properties, '$.title') AS title,

  -- Nested SEO fields
  JSON_VALUE(properties, '$.seo.title') AS seo_title,
  JSON_VALUE(properties, '$.seo.description') AS seo_desc,

  -- Deeply nested metadata
  JSON_VALUE(properties, '$.metadata.author.name') AS author_name,
  JSON_VALUE(properties, '$.metadata.author.email') AS author_email,
  JSON_VALUE(properties, '$.metadata.social.twitter') AS twitter,

  -- Check existence of nested paths
  JSON_EXISTS(properties, '$.metadata.social') AS has_social
FROM default
WHERE JSON_EXISTS(properties, '$.seo.title')
  AND node_type = 'raisin:Page'
ORDER BY path
LIMIT 20;

-- JSONPath supports:
-- $.field              - Direct field access
-- $.nested.field       - Nested field access
-- $.array[0]           - Array element access
-- $.object.array[0]    - Combined access`,
        },
        {
          title: 'Validate JSON Structure',
          description: 'Check data quality by validating required JSON fields',
          sql: `-- Data quality report: Check for required fields
SELECT
  node_type,
  COUNT(*) AS total_count,
  SUM(CASE WHEN JSON_EXISTS(properties, '$.title') THEN 1 ELSE 0 END) AS has_title,
  SUM(CASE WHEN JSON_EXISTS(properties, '$.description') THEN 1 ELSE 0 END) AS has_description,
  SUM(CASE WHEN JSON_EXISTS(properties, '$.author') THEN 1 ELSE 0 END) AS has_author,
  SUM(CASE WHEN JSON_EXISTS(properties, '$.seo.title') THEN 1 ELSE 0 END) AS has_seo_title,
  ROUND(100.0 * SUM(CASE WHEN JSON_EXISTS(properties, '$.seo.title') THEN 1 ELSE 0 END) / COUNT(*), 2) AS seo_completion_pct
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
GROUP BY node_type
ORDER BY total_count DESC;`,
        },
      ],
    },
    {
      id: 'jsonb-ops',
      title: 'JSONB Property Operations',
      icon: <Code2 className="w-4 h-4" />,
      description: 'Complete guide to modifying, merging, and manipulating JSON properties',
      examples: [
        {
          title: 'JSONB Extraction Operators',
          description: 'All methods to extract values from JSON properties',
          sql: `-- Extract JSONB object (returns JSONB type)
SELECT
  properties -> 'seo' AS seo_object,
  properties -> 'metadata' -> 'author' AS nested_object
FROM default;

-- Extract as TEXT (returns string)
SELECT
  properties ->> 'title' AS title_text,
  properties ->> 'status' AS status_text
FROM default;

-- Extract by PATH as JSONB (PostgreSQL array path)
SELECT
  properties #> '{seo,social}' AS social_obj,
  properties #> '{metadata,tags}' AS tags_array
FROM default;

-- Extract by PATH as TEXT
SELECT
  properties #>> '{seo,title}' AS seo_title,
  properties #>> '{metadata,author,name}' AS author_name
FROM default;

-- Comparison: -> vs ->> vs #> vs #>>
-- ->   : key → JSONB    (for further JSON operations)
-- ->>  : key → TEXT     (for string comparisons)
-- #>   : path → JSONB   (nested extraction as JSON)
-- #>>  : path → TEXT    (nested extraction as string)`,
        },
        {
          title: 'Key & Value Existence Checks',
          description: 'Check if keys or values exist in JSON properties',
          sql: `-- Check if key exists (? operator)
SELECT * FROM default
WHERE properties ? 'featured';

-- Check if key does NOT exist
SELECT * FROM default
WHERE NOT (properties ? 'deletedAt');

-- Check if any of multiple keys exist (?| operator)
SELECT * FROM default
WHERE properties ?| array['featured', 'promoted', 'highlighted'];

-- Check if ALL keys exist (?& operator)
SELECT * FROM default
WHERE properties ?& array['title', 'description', 'author'];

-- Check if JSON contains value (@> operator)
SELECT * FROM default
WHERE properties @> '{"status": "published"}';

-- Check if array contains element
SELECT * FROM default
WHERE properties -> 'tags' ? 'rust';

-- Check nested object contains value
SELECT * FROM default
WHERE properties -> 'seo' @> '{"indexed": true}';

-- Combine multiple checks
SELECT * FROM default
WHERE properties ? 'title'
  AND properties @> '{"status": "published"}'
  AND NOT (properties ? 'archived');`,
        },
        {
          title: 'Merge Properties (|| Operator)',
          description: 'Add or update properties using JSONB concatenation',
          sql: `-- Add/update single property
UPDATE default
SET properties = properties || '{"featured": true}'
WHERE id = 'node-id-here';

-- Add/update multiple properties at once
UPDATE default
SET properties = properties || '{"featured": true, "priority": 1, "reviewed": true}'
WHERE path = '/content/blog/post1';

-- Merge nested objects (shallow merge at top level)
UPDATE default
SET properties = properties || '{"seo": {"indexed": true}}'
WHERE id = 'node-id-here';

-- Chain multiple merges
UPDATE default
SET properties = properties
  || '{"status": "published"}'
  || '{"publishedAt": "2024-01-15T10:30:00Z"}'
  || '{"publishedBy": "editor@example.com"}'
WHERE path = '/content/article';

-- Bulk update with merge
UPDATE default
SET properties = properties || '{"migrated": true, "migratedAt": "2024-01-01"}'
WHERE PATH_STARTS_WITH(path, '/content/legacy/')
  AND NOT (properties ? 'migrated');`,
        },
        {
          title: 'Remove Properties (- and #- Operators)',
          description: 'Delete keys from JSON properties',
          sql: `-- Remove single top-level key (- operator)
UPDATE default
SET properties = properties - 'deprecated_field'
WHERE id = 'node-id-here';

-- Remove multiple keys (chain - operators)
UPDATE default
SET properties = properties - 'old_field1' - 'old_field2' - 'temp_data'
WHERE path = '/content/page';

-- Remove nested key (#- operator with path)
UPDATE default
SET properties = properties #- '{seo,oldField}'
WHERE id = 'node-id-here';

-- Remove deeply nested key
UPDATE default
SET properties = properties #- '{metadata,social,deprecated}'
WHERE path = '/content/blog/post1';

-- Remove array element by index (0-based)
UPDATE default
SET properties = properties #- '{tags,0}'  -- Removes first element
WHERE id = 'node-id-here';

-- Bulk cleanup: Remove deprecated fields from all nodes
UPDATE default
SET properties = properties - 'legacyId' - 'oldFormat'
WHERE properties ? 'legacyId'
  AND PATH_STARTS_WITH(path, '/content/');`,
        },
        {
          title: 'Set Nested Values (jsonb_set)',
          description: 'Set or create values at specific paths in JSON',
          sql: `-- Set top-level property
UPDATE default
SET properties = jsonb_set(properties, '{status}', '"published"')
WHERE id = 'node-id-here';

-- Set nested property
UPDATE default
SET properties = jsonb_set(properties, '{seo,title}', '"New SEO Title"')
WHERE path = '/content/page';

-- Set deeply nested property
UPDATE default
SET properties = jsonb_set(
  properties,
  '{metadata,social,twitter,handle}',
  '"@myhandle"'
)
WHERE id = 'node-id-here';

-- Set numeric value
UPDATE default
SET properties = jsonb_set(properties, '{views}', '100')
WHERE path = '/content/article';

-- Set boolean value
UPDATE default
SET properties = jsonb_set(properties, '{featured}', 'true')
WHERE id = 'node-id-here';

-- Set array value
UPDATE default
SET properties = jsonb_set(
  properties,
  '{tags}',
  '["rust", "database", "performance"]'
)
WHERE path = '/content/article';

-- Set object value
UPDATE default
SET properties = jsonb_set(
  properties,
  '{seo}',
  '{"title": "Page Title", "description": "Page desc", "indexed": true}'
)
WHERE id = 'node-id-here';

-- Create missing intermediate paths (4th param = true)
UPDATE default
SET properties = jsonb_set(
  properties,
  '{analytics,tracking,enabled}',
  'true',
  true  -- create_if_missing
)
WHERE id = 'node-id-here';`,
        },
        {
          title: 'Array Operations',
          description: 'Manipulate arrays within JSON properties',
          sql: `-- Access array element by index (0-based)
SELECT
  properties -> 'tags' -> 0 AS first_tag,
  properties -> 'tags' -> 1 AS second_tag,
  properties -> 'tags' -> -1 AS last_tag  -- Negative index from end
FROM default
WHERE properties ? 'tags';

-- Append to array (merge arrays with ||)
UPDATE default
SET properties = jsonb_set(
  properties,
  '{tags}',
  (properties -> 'tags') || '["new-tag"]'
)
WHERE id = 'node-id-here';

-- Prepend to array
UPDATE default
SET properties = jsonb_set(
  properties,
  '{tags}',
  '["first-tag"]' || (properties -> 'tags')
)
WHERE id = 'node-id-here';

-- Replace entire array
UPDATE default
SET properties = jsonb_set(
  properties,
  '{categories}',
  '["tech", "tutorial", "rust"]'
)
WHERE path = '/content/article';

-- Replace specific array element
UPDATE default
SET properties = jsonb_set(
  properties,
  '{tags,0}',  -- First element
  '"updated-tag"'
)
WHERE id = 'node-id-here';

-- Remove array element by index
UPDATE default
SET properties = properties #- '{tags,2}'  -- Remove 3rd element
WHERE id = 'node-id-here';

-- Check array length in WHERE
SELECT * FROM default
WHERE jsonb_array_length(properties -> 'tags') > 3;`,
        },
        {
          title: 'Type Conversions & Comparisons',
          description: 'Cast JSON values for comparisons and calculations',
          sql: `-- Cast to TEXT for string comparison
SELECT * FROM default
WHERE properties ->> 'status' = 'published';

-- Cast to INTEGER for numeric comparison
SELECT * FROM default
WHERE (properties ->> 'views')::int > 1000;

-- Cast to NUMERIC/DOUBLE for decimal comparison
SELECT * FROM default
WHERE (properties ->> 'price')::numeric > 99.99;

-- Cast to BOOLEAN for boolean comparison
SELECT * FROM default
WHERE (properties ->> 'featured')::boolean = true;

-- Cast to TIMESTAMP for date comparison
SELECT * FROM default
WHERE (properties ->> 'publishedAt')::timestamp > '2024-01-01';

-- Use in ORDER BY with casting
SELECT id, name, (properties ->> 'views')::int AS views
FROM default
WHERE properties ? 'views'
ORDER BY (properties ->> 'views')::int DESC;

-- Use in calculations
SELECT
  name,
  (properties ->> 'price')::numeric AS price,
  (properties ->> 'quantity')::int AS qty,
  (properties ->> 'price')::numeric * (properties ->> 'quantity')::int AS total
FROM default
WHERE node_type = 'shop:Product';

-- Convert value TO JSON (for jsonb_set)
UPDATE default
SET properties = jsonb_set(
  properties,
  '{totalViews}',
  to_jsonb((properties ->> 'views')::int + 100)
)
WHERE id = 'node-id-here';`,
        },
        {
          title: 'Increment & Toggle Patterns',
          description: 'Common patterns for updating numeric and boolean values',
          sql: `-- Increment a counter
UPDATE default
SET properties = jsonb_set(
  properties,
  '{views}',
  to_jsonb((properties ->> 'views')::int + 1)
)
WHERE id = 'node-id-here';

-- Decrement a counter (with floor at 0)
UPDATE default
SET properties = jsonb_set(
  properties,
  '{stock}',
  to_jsonb(GREATEST(0, (properties ->> 'stock')::int - 1))
)
WHERE id = 'node-id-here';

-- Add to a value
UPDATE default
SET properties = jsonb_set(
  properties,
  '{score}',
  to_jsonb((properties ->> 'score')::numeric + 10.5)
)
WHERE id = 'node-id-here';

-- Toggle a boolean
UPDATE default
SET properties = jsonb_set(
  properties,
  '{featured}',
  to_jsonb(NOT (properties ->> 'featured')::boolean)
)
WHERE id = 'node-id-here';

-- Set boolean based on condition
UPDATE default
SET properties = jsonb_set(
  properties,
  '{popular}',
  to_jsonb((properties ->> 'views')::int > 1000)
)
WHERE properties ? 'views';

-- Increment nested counter
UPDATE default
SET properties = jsonb_set(
  properties,
  '{metadata,visitCount}',
  to_jsonb((properties #>> '{metadata,visitCount}')::int + 1)
)
WHERE id = 'node-id-here';`,
        },
        {
          title: 'Conditional JSON Updates',
          description: 'Update properties based on existing JSON values',
          sql: `-- Update only if key exists
UPDATE default
SET properties = properties || '{"lastModified": "2024-01-15"}'
WHERE properties ? 'createdAt';

-- Update only if key does NOT exist (initialize)
UPDATE default
SET properties = properties || '{"views": 0}'
WHERE NOT (properties ? 'views')
  AND node_type = 'raisin:Page';

-- Update based on JSON value match
UPDATE default
SET properties = properties || '{"featured": true}'
WHERE properties ->> 'status' = 'published'
  AND (properties ->> 'views')::int > 1000;

-- Update based on nested value
UPDATE default
SET properties = jsonb_set(properties, '{seo,indexed}', 'true')
WHERE properties -> 'seo' ->> 'title' IS NOT NULL
  AND node_type = 'raisin:Page';

-- Update based on array containment
UPDATE default
SET properties = properties || '{"isRustContent": true}'
WHERE properties -> 'tags' ? 'rust';

-- Bulk conditional update
UPDATE default
SET properties = properties || '{"needsReview": true}'
WHERE properties ->> 'status' = 'draft'
  AND (properties ->> 'updatedAt')::timestamp < '2024-01-01';

-- Update with CASE expression
UPDATE default
SET properties = jsonb_set(
  properties,
  '{tier}',
  CASE
    WHEN (properties ->> 'views')::int > 10000 THEN '"gold"'
    WHEN (properties ->> 'views')::int > 1000 THEN '"silver"'
    ELSE '"bronze"'
  END::jsonb
)
WHERE properties ? 'views';`,
        },
        {
          title: 'Filter & Query by JSON',
          description: 'WHERE clause patterns for JSON filtering',
          sql: `-- Simple equality
SELECT * FROM default
WHERE properties ->> 'status' = 'published';

-- Pattern matching with LIKE
SELECT * FROM default
WHERE properties ->> 'title' LIKE '%tutorial%';

-- Case-insensitive search
SELECT * FROM default
WHERE LOWER(properties ->> 'title') LIKE '%rust%';

-- IN clause with JSON values
SELECT * FROM default
WHERE properties ->> 'status' IN ('draft', 'review', 'published');

-- NULL checks
SELECT * FROM default
WHERE properties ->> 'deletedAt' IS NULL;  -- Key exists but value is null
-- vs
WHERE NOT (properties ? 'deletedAt');       -- Key doesn't exist at all

-- Numeric range
SELECT * FROM default
WHERE (properties ->> 'price')::numeric BETWEEN 10 AND 100;

-- Date range
SELECT * FROM default
WHERE (properties ->> 'publishedAt')::date >= '2024-01-01'
  AND (properties ->> 'publishedAt')::date < '2024-02-01';

-- Complex boolean logic
SELECT * FROM default
WHERE (
  properties @> '{"status": "published"}'
  OR properties @> '{"featured": true}'
)
AND properties ? 'title'
AND NOT (properties ? 'archived');

-- Full-text search combined with JSON filter
SELECT * FROM default
WHERE FULLTEXT_MATCH('database performance', 'english')
  AND properties ->> 'status' = 'published'
  AND (properties ->> 'views')::int > 100;`,
        },
        {
          title: 'JSONB Operator Quick Reference',
          description: 'Complete reference of all JSONB operators',
          sql: `-- ═══════════════════════════════════════════════════════════
-- JSONB OPERATOR QUICK REFERENCE
-- ═══════════════════════════════════════════════════════════

-- EXTRACTION OPERATORS:
-- ->   Extract JSONB object/array by key
-- ->>  Extract as TEXT by key
-- #>   Extract JSONB by path array
-- #>>  Extract TEXT by path array

SELECT
  properties -> 'seo'                    AS "-> JSONB by key",
  properties ->> 'title'                 AS "->> TEXT by key",
  properties #> '{seo,social}'           AS "#> JSONB by path",
  properties #>> '{seo,title}'           AS "#>> TEXT by path"
FROM default LIMIT 1;

-- EXISTENCE OPERATORS:
-- ?    Key exists
-- ?|   Any key exists (from array)
-- ?&   All keys exist (from array)
-- @>   JSON contains value

SELECT *
FROM default
WHERE properties ? 'featured'                         -- "?" key exists
  AND properties ?| array['tag1','tag2']              -- "?|" any exists
  AND properties ?& array['title','status']           -- "?&" all exist
  AND properties @> '{"status":"published"}';         -- "@>" contains

-- MODIFICATION OPERATORS (for UPDATE):
-- ||   Concatenate/merge JSONB
-- -    Remove key (by name)
-- #-   Remove at path

UPDATE default SET properties =
  properties
  || '{"new": "value"}'        -- "||" merge
  - 'oldKey'                   -- "-" remove key
  #- '{nested,oldKey}'         -- "#-" remove at path
WHERE id = '...';

-- FUNCTIONS:
-- jsonb_set(json, path, value, create_missing)
-- to_jsonb(value)
-- jsonb_array_length(array)

UPDATE default SET properties =
  jsonb_set(properties, '{views}', to_jsonb(100))
WHERE id = '...';`,
        },
      ],
    },
    {
      id: 'translations',
      title: 'Translations & Locales',
      icon: <Globe className="w-4 h-4" />,
      description: 'Query translated content with locale filtering using WHERE clause',
      examples: [
        {
          title: 'Query Single Locale',
          description: 'Get nodes with specific locale translations using WHERE clause',
          sql: `-- Get all nodes with French translations
SELECT
  id,
  name,
  path,
  locale,  -- Virtual column showing resolved locale (always 'fr' here)
  properties ->> 'title' AS title,
  properties ->> 'description' AS description
FROM default
WHERE locale = 'fr'
  AND PATH_STARTS_WITH(path, '/content/')
ORDER BY name
LIMIT 20;

-- Returns nodes with French translations applied
-- Falls back to default language if no translation exists`,
        },
        {
          title: 'Multi-Locale Query (Multiple Rows per Node)',
          description: 'Get the same nodes in multiple locales - returns one row per locale per node',
          sql: `-- Get nodes in both English and German
-- This returns DUPLICATE rows: one per locale per node
SELECT
  id,
  name,
  path,
  locale,
  properties ->> 'title' AS title
FROM default
WHERE locale IN ('en', 'de')
  AND PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY path, locale;

-- Result example:
-- id       | path              | locale | title
-- ---------|-------------------|--------|-------------
-- node-123 | /content/blog/... | en     | My Blog Post
-- node-123 | /content/blog/... | de     | Mein Blogbeitrag
-- node-456 | /content/about    | en     | About Us
-- node-456 | /content/about    | de     | Über uns`,
        },
        {
          title: 'Default Language (No Locale Filter)',
          description: 'Query without locale filter uses repository default language',
          sql: `-- No locale filter - uses repository default (typically 'en')
SELECT
  id,
  name,
  path,
  locale,  -- Will show default language (e.g., 'en')
  properties ->> 'title' AS title
FROM default
WHERE node_type = 'raisin:Page'
  AND PATH_STARTS_WITH(path, '/content/')
ORDER BY properties ->> 'published_date' DESC
LIMIT 20;

-- Returns nodes in default language only
-- locale column shows the default (e.g., 'en')`,
        },
        {
          title: 'Translation Coverage Report',
          description: 'Export all content in multiple languages for translation analysis',
          sql: `-- Get all locales for content audit
-- Returns multiple rows per node
SELECT
  id,
  path,
  locale,
  node_type,
  properties ->> 'title' AS title,
  properties ->> 'status' AS status
FROM default
WHERE locale IN ('en', 'de', 'fr', 'es')
  AND PATH_STARTS_WITH(path, '/content/')
ORDER BY path, locale;

-- Great for identifying missing translations
-- Export to CSV and analyze coverage per locale`,
        },
        {
          title: 'Compare Translations Side-by-Side',
          description: 'Join same nodes with different locales to compare translations',
          sql: `-- Compare English vs German translations
SELECT
  e.id,
  e.path,
  e.properties ->> 'title' AS title_en,
  d.properties ->> 'title' AS title_de,
  e.properties ->> 'status' AS status
FROM (SELECT * FROM default WHERE locale = 'en') e
JOIN (SELECT * FROM default WHERE locale = 'de') d ON d.id = e.id
WHERE e.node_type = 'raisin:Page'
  AND e.PATH_STARTS_WITH(e.path, '/content/')
ORDER BY e.path
LIMIT 20;`,
        },
        {
          title: 'Find Missing Translations',
          description: 'Identify nodes that lack specific locale translations',
          sql: `-- Find nodes that exist in English but not in German
WITH english_nodes AS (
  SELECT id, path FROM default WHERE locale = 'en'
),
german_nodes AS (
  SELECT id FROM default WHERE locale = 'de'
)
SELECT
  e.id,
  e.path,
  'Missing German translation' AS issue
FROM english_nodes e
LEFT JOIN german_nodes d ON d.id = e.id
WHERE d.id IS NULL
  AND PATH_STARTS_WITH(e.path, '/content/')
ORDER BY e.path;`,
        },
        {
          title: 'Multi-Locale Full-Text Search',
          description: 'Search across different languages with language-specific stemming',
          sql: `-- Search German content with German stemming
SELECT
  id,
  name,
  path,
  locale,
  properties ->> 'title' AS title,
  properties ->> 'body' AS body_preview
FROM default
WHERE locale = 'de'
  AND FULLTEXT_MATCH('datenbank AND leistung', 'german')
  AND properties ->> 'status' = 'published'
ORDER BY updated_at DESC
LIMIT 20;

-- For English: WHERE locale = 'en' AND FULLTEXT_MATCH('database AND performance', 'english')`,
        },
        {
          title: 'Locale-Aware Hierarchical Query',
          description: 'Get entire content tree with specific locale translations',
          sql: `-- Get French translations of entire blog subtree
SELECT
  id,
  name,
  path,
  locale,
  DEPTH(path) AS depth_level,
  PARENT(path) AS parent_path,
  properties ->> 'title' AS title,
  properties ->> 'status' AS status
FROM default
WHERE locale = 'fr'
  AND PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY path;

-- All nodes in subtree with French translations
-- Falls back to default language if French not available`,
        },
        {
          title: 'Create/Update Single Translation',
          description: 'Use UPDATE FOR LOCALE to add or update translations for a specific locale',
          sql: `-- Translate a single field to German
UPDATE default FOR LOCALE 'de'
SET bio = 'Biografie auf Deutsch'
WHERE path = '/users/senol';

-- This creates or updates the German translation overlay
-- The base node properties remain unchanged
-- Query with locale='de' to see translated content`,
        },
        {
          title: 'Translate Multiple Fields',
          description: 'Update multiple properties in a single translation statement',
          sql: `-- Translate multiple fields at once using comma-separated SET
UPDATE default FOR LOCALE 'de'
SET bio = 'Biografie auf Deutsch',
    displayName = 'Senol Anzeigename',
    title = 'Deutscher Titel'
WHERE path = '/users/senol';

-- All specified properties are translated in one operation
-- Untranslated properties fall back to default locale`,
        },
        {
          title: 'Translate in a Specific Branch',
          description: 'Use IN BRANCH clause to create translations in a non-main branch',
          sql: `-- Create translation in a feature branch
UPDATE default FOR LOCALE 'de' IN BRANCH 'localization-sprint'
SET bio = 'Neue deutsche Bio',
    displayName = 'Neuer Anzeigename'
WHERE path = '/users/senol';

-- Translation is stored in 'localization-sprint' branch only
-- Main branch translations remain unchanged
-- Can be merged later via branch operations`,
        },
        {
          title: 'Translate with Transaction',
          description: 'Wrap multiple translations in a transaction for atomic updates',
          sql: `-- Batch translate multiple nodes atomically
BEGIN;

UPDATE default FOR LOCALE 'de' IN BRANCH 'localization-sprint'
SET title = 'Startseite', description = 'Willkommen'
WHERE path = '/content/home';

UPDATE default FOR LOCALE 'de' IN BRANCH 'localization-sprint'
SET title = 'Über uns', description = 'Unser Team'
WHERE path = '/content/about';

COMMIT WITH MESSAGE 'Translated homepage and about page to German';

-- All translations are committed together
-- If any fails, all are rolled back`,
        },
      ],
    },
    {
      id: 'window',
      title: 'Window Functions & Analytics',
      icon: <Sparkles className="w-4 h-4" />,
      description: 'Analytical queries with ROW_NUMBER, RANK, running totals, and partitioned aggregates',
      examples: [
        {
          title: 'ROW_NUMBER for Ranking',
          description: 'Assign sequential row numbers within partitions',
          sql: `-- Number rows within each parent path
SELECT
  id,
  name,
  path,
  PARENT(path) AS parent,
  ROW_NUMBER() OVER (
    PARTITION BY PARENT(path)
    ORDER BY name
  ) AS row_num
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
  AND DEPTH(path) = 3
ORDER BY PARENT(path), row_num;

-- Find position of each node among its siblings`,
        },
        {
          title: 'RANK and DENSE_RANK',
          description: 'Rank items with gap handling for ties',
          sql: `-- Rank blog posts by views, handling ties
SELECT
  id,
  name,
  path,
  CAST(properties ->> 'views' AS INT) AS views,
  RANK() OVER (ORDER BY CAST(properties ->> 'views' AS INT) DESC) AS rank,
  DENSE_RANK() OVER (ORDER BY CAST(properties ->> 'views' AS INT) DESC) AS dense_rank
FROM default
WHERE node_type = 'raisin:Page'
  AND properties ? 'views'
ORDER BY views DESC
LIMIT 20;

-- RANK: 1, 2, 2, 4 (gaps after ties)
-- DENSE_RANK: 1, 2, 2, 3 (no gaps)`,
        },
        {
          title: 'Running Totals',
          description: 'Calculate cumulative sums using window frames',
          sql: `-- Running total of content items by date
SELECT
  properties ->> 'created_date' AS created_date,
  name,
  path,
  COUNT(*) OVER (
    ORDER BY properties ->> 'created_date'
    ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
  ) AS cumulative_count
FROM default
WHERE node_type = 'raisin:Page'
  AND properties ? 'created_date'
ORDER BY properties ->> 'created_date';`,
        },
        {
          title: 'Top N per Group',
          description: 'Get top 3 items from each category using ROW_NUMBER',
          sql: `-- Top 3 most recent posts per category
WITH ranked AS (
  SELECT
    id,
    name,
    path,
    ANCESTOR(path, 2) AS category,
    properties ->> 'published_date' AS published_date,
    ROW_NUMBER() OVER (
      PARTITION BY ANCESTOR(path, 2)
      ORDER BY properties ->> 'published_date' DESC
    ) AS rank
  FROM default
  WHERE PATH_STARTS_WITH(path, '/content/blog/')
    AND DEPTH(path) >= 3
)
SELECT category, name, published_date
FROM ranked
WHERE rank <= 3
ORDER BY category, rank;`,
        },
        {
          title: 'Moving Average',
          description: 'Calculate moving averages with sliding window frames',
          sql: `-- 7-day moving average of daily content creation
SELECT
  properties ->> 'created_date' AS date,
  COUNT(*) AS daily_count,
  AVG(COUNT(*)) OVER (
    ORDER BY properties ->> 'created_date'
    ROWS BETWEEN 6 PRECEDING AND CURRENT ROW
  ) AS moving_avg_7day
FROM default
WHERE properties ? 'created_date'
GROUP BY properties ->> 'created_date'
ORDER BY date DESC
LIMIT 30;`,
        },
        {
          title: 'Partition by Multiple Columns',
          description: 'Use multiple partition columns for fine-grained grouping',
          sql: `-- Rank within language and category
SELECT
  id,
  name,
  path,
  locale,
  ANCESTOR(path, 2) AS category,
  CAST(properties ->> 'views' AS INT) AS views,
  ROW_NUMBER() OVER (
    PARTITION BY locale, ANCESTOR(path, 2)
    ORDER BY CAST(properties ->> 'views' AS INT) DESC
  ) AS rank_in_category
FROM default
WHERE locale IN ('en', 'de')
  AND PATH_STARTS_WITH(path, '/content/')
  AND properties ? 'views'
ORDER BY locale, category, rank_in_category;`,
        },
        {
          title: 'Window Aggregates (COUNT, SUM, AVG)',
          description: 'Aggregate over partitions without GROUP BY collapsing',
          sql: `-- Show each node with its parent's child count
SELECT
  id,
  name,
  path,
  PARENT(path) AS parent,
  COUNT(*) OVER (PARTITION BY PARENT(path)) AS sibling_count,
  SUM(CAST(properties ->> 'size' AS INT)) OVER (
    PARTITION BY PARENT(path)
  ) AS total_parent_size,
  AVG(CAST(properties ->> 'size' AS INT)) OVER (
    PARTITION BY PARENT(path)
  ) AS avg_sibling_size
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
  AND properties ? 'size'
ORDER BY path;`,
        },
        {
          title: 'MIN/MAX Window Functions',
          description: 'Find minimum and maximum values within partitions',
          sql: `-- Compare each node to min/max in its category
SELECT
  id,
  name,
  path,
  ANCESTOR(path, 2) AS category,
  CAST(properties ->> 'score' AS INT) AS score,
  MIN(CAST(properties ->> 'score' AS INT)) OVER (
    PARTITION BY ANCESTOR(path, 2)
  ) AS min_score_in_category,
  MAX(CAST(properties ->> 'score' AS INT)) OVER (
    PARTITION BY ANCESTOR(path, 2)
  ) AS max_score_in_category
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
  AND properties ? 'score'
ORDER BY category, score DESC;`,
        },
        {
          title: 'RANGE vs ROWS Frame Mode',
          description: 'Understand the difference between RANGE and ROWS frames',
          sql: `-- ROWS: Physical row offset (count rows)
SELECT
  name,
  CAST(properties ->> 'created_at' AS BIGINT) AS created_at,
  COUNT(*) OVER (
    ORDER BY CAST(properties ->> 'created_at' AS BIGINT)
    ROWS BETWEEN 2 PRECEDING AND 2 FOLLOWING
  ) AS count_rows_window
FROM default
WHERE properties ? 'created_at'
LIMIT 10;

-- RANGE: Logical value range (same value = same frame)
-- Note: RANGE requires ORDER BY on a single sortable column
SELECT
  name,
  DEPTH(path) AS depth,
  COUNT(*) OVER (
    ORDER BY DEPTH(path)
    RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
  ) AS count_up_to_depth
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
ORDER BY depth;`,
        },
        {
          title: 'Hierarchical Window Analysis',
          description: 'Combine window functions with hierarchy queries',
          sql: `-- Analyze content distribution by depth with percentages
SELECT
  DEPTH(path) AS depth_level,
  COUNT(*) AS node_count,
  SUM(COUNT(*)) OVER (
    ORDER BY DEPTH(path)
    ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
  ) AS cumulative_count,
  ROUND(100.0 * COUNT(*) / SUM(COUNT(*)) OVER (), 2) AS percentage_of_total
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
GROUP BY DEPTH(path)
ORDER BY depth_level;`,
        },
      ],
    },
    {
      id: 'subqueries',
      title: 'Subqueries & CTEs',
      icon: <Code2 className="w-4 h-4" />,
      description: 'Use subqueries in FROM clause and Common Table Expressions (WITH) for complex queries',
      examples: [
        {
          title: 'Subquery in FROM Clause',
          description: 'Use derived tables (subqueries) as data sources',
          sql: `-- Query a filtered subset as a derived table
SELECT
  sub.category,
  sub.post_count,
  sub.avg_views
FROM (
  SELECT
    ANCESTOR(path, 2) AS category,
    COUNT(*) AS post_count,
    AVG(CAST(properties ->> 'views' AS INT)) AS avg_views
  FROM default
  WHERE PATH_STARTS_WITH(path, '/content/blog/')
    AND properties ? 'views'
  GROUP BY ANCESTOR(path, 2)
) AS sub
WHERE sub.post_count > 5
ORDER BY sub.avg_views DESC;`,
        },
        {
          title: 'Common Table Expression (CTE)',
          description: 'Use WITH clause to create named temporary result sets',
          sql: `-- Define reusable CTE for published pages
WITH published_pages AS (
  SELECT
    id,
    name,
    path,
    properties ->> 'author' AS author,
    CAST(properties ->> 'views' AS INT) AS views
  FROM default
  WHERE node_type = 'raisin:Page'
    AND properties ->> 'status' = 'published'
)
SELECT
  author,
  COUNT(*) AS page_count,
  SUM(views) AS total_views,
  AVG(views) AS avg_views
FROM published_pages
GROUP BY author
ORDER BY total_views DESC
LIMIT 10;`,
        },
        {
          title: 'Multiple CTEs',
          description: 'Chain multiple CTEs for complex transformations',
          sql: `-- Multiple CTEs for step-by-step transformation
WITH blog_posts AS (
  SELECT
    id,
    name,
    path,
    ANCESTOR(path, 2) AS category,
    CAST(properties ->> 'views' AS INT) AS views
  FROM default
  WHERE PATH_STARTS_WITH(path, '/content/blog/')
    AND properties ? 'views'
),
category_stats AS (
  SELECT
    category,
    COUNT(*) AS post_count,
    SUM(views) AS total_views,
    AVG(views) AS avg_views
  FROM blog_posts
  GROUP BY category
)
SELECT
  cs.category,
  cs.post_count,
  cs.total_views,
  ROUND(cs.avg_views, 2) AS avg_views,
  ROUND(100.0 * cs.total_views / SUM(cs.total_views) OVER (), 2) AS pct_of_total
FROM category_stats cs
ORDER BY cs.total_views DESC;`,
        },
        {
          title: 'JOIN Subqueries',
          description: 'Join derived tables for complex analysis',
          sql: `-- Join subquery results
SELECT
  authors.author,
  authors.post_count,
  categories.category_count
FROM (
  SELECT
    properties ->> 'author' AS author,
    COUNT(*) AS post_count
  FROM default
  WHERE node_type = 'raisin:Page'
  GROUP BY properties ->> 'author'
) AS authors
LEFT JOIN (
  SELECT
    properties ->> 'author' AS author,
    COUNT(DISTINCT ANCESTOR(path, 2)) AS category_count
  FROM default
  WHERE PATH_STARTS_WITH(path, '/content/')
  GROUP BY properties ->> 'author'
) AS categories ON categories.author = authors.author
ORDER BY authors.post_count DESC
LIMIT 20;`,
        },
        {
          title: 'Correlated Subquery in WHERE',
          description: 'Use correlated subqueries for row-by-row filtering',
          sql: `-- Find nodes with above-average views in their category
WITH category_averages AS (
  SELECT
    ANCESTOR(path, 2) AS category,
    AVG(CAST(properties ->> 'views' AS INT)) AS avg_views
  FROM default
  WHERE PATH_STARTS_WITH(path, '/content/blog/')
    AND properties ? 'views'
  GROUP BY ANCESTOR(path, 2)
)
SELECT
  d.id,
  d.name,
  d.path,
  ANCESTOR(d.path, 2) AS category,
  CAST(d.properties ->> 'views' AS INT) AS views,
  ca.avg_views AS category_avg
FROM default d
JOIN category_averages ca ON ca.category = ANCESTOR(d.path, 2)
WHERE CAST(d.properties ->> 'views' AS INT) > ca.avg_views
ORDER BY category, views DESC;`,
        },
        {
          title: 'Recursive-Style Query with CTEs',
          description: 'Simulate hierarchical traversal using CTEs',
          sql: `-- Get parent and grandparent information using CTEs
WITH parent_info AS (
  SELECT
    id,
    name,
    path,
    PARENT(path) AS parent_path
  FROM default
  WHERE PATH_STARTS_WITH(path, '/content/blog/')
)
SELECT
  p.id,
  p.name AS node_name,
  p.path AS node_path,
  parent.name AS parent_name,
  grandparent.name AS grandparent_name
FROM parent_info p
LEFT JOIN default parent ON parent.path = p.parent_path
LEFT JOIN default grandparent ON grandparent.path = PARENT(parent.path)
WHERE p.path IS NOT NULL
ORDER BY p.path
LIMIT 20;`,
        },
        {
          title: 'Subquery with Window Functions',
          description: 'Combine subqueries with window functions for advanced analytics',
          sql: `-- Rank categories by total views, then show top posts per category
WITH category_ranks AS (
  SELECT
    ANCESTOR(path, 2) AS category,
    SUM(CAST(properties ->> 'views' AS INT)) AS total_views,
    RANK() OVER (ORDER BY SUM(CAST(properties ->> 'views' AS INT)) DESC) AS category_rank
  FROM default
  WHERE PATH_STARTS_WITH(path, '/content/blog/')
    AND properties ? 'views'
  GROUP BY ANCESTOR(path, 2)
)
SELECT
  cr.category,
  cr.total_views,
  cr.category_rank,
  d.name AS post_name,
  d.path AS post_path,
  CAST(d.properties ->> 'views' AS INT) AS post_views
FROM category_ranks cr
JOIN default d ON ANCESTOR(d.path, 2) = cr.category
WHERE cr.category_rank <= 5
  AND d.properties ? 'views'
ORDER BY cr.category_rank, post_views DESC;`,
        },
      ],
    },
    {
      id: 'patterns',
      title: 'Common SQL Patterns',
      icon: <Code2 className="w-4 h-4" />,
      description: 'Practical patterns for joins, pagination, filtering, and aggregation',
      examples: [
        {
          title: 'JSON Property Filtering',
          description: 'Filter nodes by JSON property values',
          sql: `-- Filter by JSON properties
SELECT id, name, path, properties
FROM default
WHERE properties ->> 'status' = 'published'
  AND (properties ->> 'author')::TEXT LIKE '%smith%'
  AND properties @> '{"featured": true}'
ORDER BY properties ->> 'published_date' DESC;`,
        },
        {
          title: 'Pagination (Offset-Based)',
          description: 'Traditional pagination with LIMIT and OFFSET',
          sql: `-- Page 3, 20 items per page
SELECT id, name, path, properties
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
ORDER BY properties ->> 'created_at' DESC
LIMIT 20 OFFSET 40;`,
        },
        {
          title: 'Pagination (Cursor-Based)',
          description: 'More efficient pagination using WHERE comparison',
          sql: `-- Cursor-based pagination (pass last_id from previous page)
SELECT id, name, path, properties
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
  AND id > 'last_id_from_previous_page'
ORDER BY id
LIMIT 20;`,
        },
        {
          title: 'Hierarchical Aggregation',
          description: 'Aggregate statistics by hierarchy level',
          sql: `-- Count nodes per hierarchy level
SELECT
  DEPTH(path) AS depth_level,
  COUNT(*) AS node_count,
  COUNT(DISTINCT PARENT(path)) AS unique_parents
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
GROUP BY DEPTH(path)
ORDER BY depth_level;`,
        },
        {
          title: 'Filter by Node Type',
          description: 'Query specific node types with property filtering',
          sql: `-- Get all published pages under a path
SELECT id, name, path, properties
FROM default
WHERE node_type = 'raisin:Page'
  AND PATH_STARTS_WITH(path, '/content/blog/')
  AND properties ->> 'status' = 'published'
ORDER BY properties ->> 'published_date' DESC;`,
        },
        {
          title: 'Complex JSON Filtering',
          description: 'Query nested JSON structures and arrays',
          sql: `-- Filter by nested JSON and array containment
SELECT id, name, path, properties
FROM default
WHERE properties -> 'metadata' ->> 'category' = 'tutorial'
  AND properties @> '{"tags": ["rust"]}'
  AND properties -> 'seo' ? 'description'
ORDER BY name;`,
        },
        {
          title: 'Aggregate with HAVING',
          description: 'Group by and filter aggregated results',
          sql: `-- Find parent nodes with more than 5 children
SELECT
  PARENT(path) AS parent_path,
  COUNT(*) AS child_count,
  ARRAY_AGG(name) AS child_names
FROM default
WHERE PARENT(path) IS NOT NULL
GROUP BY PARENT(path)
HAVING COUNT(*) > 5
ORDER BY child_count DESC;`,
        },
      ],
    },
    {
      id: 'revisions',
      title: 'Revisions & Branches',
      icon: <GitBranch className="w-4 h-4" />,
      description: 'Query different revisions and branches (point-in-time queries)',
      examples: [
        {
          title: 'Understanding Revisions',
          description: 'RaisinDB stores all node versions. Query specific points in time.',
          sql: `-- Current implementation: Revisions are set via API context
-- The SQL engine receives max_revision parameter from execution context
--
-- Example API usage:
-- POST /api/repository/${repo}/sql?max_revision=12345
-- Body: { "query": "SELECT * FROM default" }
--
-- This returns the state of all nodes at revision 12345

-- In SQL, you see the latest version by default (HEAD)
SELECT id, name, path, version, properties
FROM default
WHERE path = '/content/blog'
-- Returns latest version (HEAD)`,
        },
        {
          title: 'Branch Queries',
          description: 'Query different branches via API context',
          sql: `-- Branches are also set via API context, not SQL syntax
-- Each repository can have multiple branches
--
-- Example API usage:
-- POST /api/repository/${repo}/sql?branch=feature-branch
-- Body: { "query": "SELECT * FROM default" }
--
-- Default branch is typically 'main' or 'master'

-- Your SQL remains the same across branches:
SELECT id, name, path, node_type
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
-- Results depend on active branch context`,
        },
        {
          title: 'Query Node History',
          description: 'Note: Full history queries require storage API, not SQL',
          sql: `-- Current SQL queries always return latest version per node
-- To query full history of a node, use the storage API:
--
-- GET /api/repository/${repo}/nodes?path=/content/blog&history=true
--
-- SQL sees only latest version:
SELECT id, name, path, version, properties
FROM default
WHERE path = '/content/blog';
-- Returns: version=latest only`,
        },
        {
          title: 'SELECT from Specific Branch',
          description: 'Use __branch virtual column in WHERE clause to query a specific branch',
          sql: `-- Query nodes from a feature branch
SELECT id, name, path, properties
FROM default
WHERE __branch = 'feature-redesign'
  AND PATH_STARTS_WITH(path, '/content/')
ORDER BY path;

-- Compare content between branches
SELECT
  main.path,
  main.properties ->> 'title' AS main_title,
  feat.properties ->> 'title' AS feature_title
FROM (SELECT * FROM default WHERE __branch = 'main') main
LEFT JOIN (SELECT * FROM default WHERE __branch = 'feature-redesign') feat
  ON feat.id = main.id
WHERE main.node_type = 'raisin:Page';`,
        },
        {
          title: 'UPDATE/DELETE in Branch',
          description: 'Use __branch in WHERE clause for standard UPDATE and DELETE statements',
          sql: `-- Update a node in a specific branch
UPDATE default
SET properties = jsonb_set(properties, '{status}', '"draft"')
WHERE path = '/content/article'
  AND __branch = 'editorial-review';

-- Delete nodes from a feature branch only
DELETE FROM default
WHERE PATH_STARTS_WITH(path, '/content/temp/')
  AND __branch = 'cleanup-branch';

-- Main branch is unaffected by these operations`,
        },
        {
          title: 'MOVE/ORDER in Branch',
          description: 'Use IN BRANCH clause for tree restructuring operations',
          sql: `-- Move a node to new parent in a feature branch
MOVE default IN BRANCH 'nav-restructure'
SET path = '/content/about' TO path = '/about-us';

-- Reorder sibling nodes in a staging branch
ORDER default IN BRANCH 'staging'
SET path = '/blog/latest-post' ABOVE path = '/blog/older-post';

-- These changes only affect the specified branch
-- Main branch tree structure remains unchanged`,
        },
        {
          title: 'Branch-Aware Transactions',
          description: 'Combine multiple branch operations in a transaction',
          sql: `-- Atomic operations across branches
BEGIN;

-- Update content in feature branch
UPDATE default
SET properties = jsonb_set(properties, '{title}', '"New Title"')
WHERE path = '/content/home' AND __branch = 'redesign';

-- Reorder navigation in same branch
ORDER default IN BRANCH 'redesign'
SET path = '/nav/contact' ABOVE path = '/nav/about';

COMMIT WITH MESSAGE 'Updated homepage and reordered nav';

-- All changes are committed atomically to the branch`,
        },
      ],
    },
    {
      id: 'cypher',
      title: 'Graph Queries (Cypher)',
      icon: <Sparkles className="w-4 h-4" />,
      description: 'Query graph relationships and run graph algorithms using Cypher',
      examples: [
        {
          title: 'Path-First MATCH (recommended)',
          description: 'Use CYPHER() to grab IDs, then hydrate nodes with SQL joins',
          sql: `-- Fetch related articles with metadata via Cypher + SQL
WITH graph_edges AS (
  SELECT cypher.target_id,
         cypher.relation_type,
         COALESCE(cypher.weight, 0.75) AS weight
  FROM CYPHER('
    MATCH (this)-[r]->(n)
    WHERE this.path = "/superbigshit/articles/tech/rust-web-development-2025"
      AND type(r) IN ["similar-to", "see-also", "updates"]
    RETURN n.id AS target_id, type(r) AS relation_type, r.weight AS weight
    LIMIT 5
  ') AS cypher(target_id UUID, relation_type TEXT, weight DOUBLE PRECISION)
)
SELECT s.id, s.path, s.name, graph_edges.relation_type, graph_edges.weight
FROM graph_edges
JOIN social s ON s.id = graph_edges.target_id
ORDER BY graph_edges.weight DESC;`,
        },
        {
          title: 'Traverse Relationships',
          description: 'Follow specific relationship types',
          sql: `-- Follow LINK relationships
SELECT * FROM CYPHER('
  MATCH (a:Page)-[:LINK]->(b:Page)
  WHERE a.path STARTS WITH "/content"
  RETURN a.name AS from_page, b.name AS to_page
');`,
        },
        {
          title: 'Graph Neighbors',
          description: 'Get neighboring nodes via table function',
          sql: `-- Get outgoing neighbors of a node
SELECT
  n.id,
  n.name,
  n.path,
  neighbors.relation_type
FROM NEIGHBORS('node_id_here', 'OUT', 'LINK') AS neighbors
JOIN default n ON n.id = neighbors.neighbor_id;`,
        },
        {
          title: 'PageRank Algorithm',
          description: 'Run PageRank to find important nodes in graph',
          sql: `-- Calculate PageRank for linked pages
SELECT * FROM CYPHER('
  CALL algo.pageRank()
  YIELD node, score
  RETURN node.path, node.name, score
  ORDER BY score DESC
  LIMIT 20
');`,
        },
        {
          title: 'Shortest Path',
          description: 'Find shortest path between two nodes',
          sql: `-- Find shortest path between nodes
SELECT * FROM CYPHER('
  MATCH path = shortestPath(
    (a)-[*]-(b)
  )
  WHERE a.path = "/start" AND b.path = "/end"
  RETURN path, length(path) AS hop_count
');`,
        },
        {
          title: 'JOIN Cypher with SQL',
          description: 'Combine graph queries with SQL tables',
          sql: `-- Combine Cypher graph results with SQL filtering
WITH graph_results AS (
  SELECT * FROM CYPHER('
    MATCH (a)-[:LINK]->(b)
    RETURN a.id AS source_id, b.id AS target_id
  ')
)
SELECT
  gr.source_id,
  n1.name AS source_name,
  gr.target_id,
  n2.name AS target_name,
  n2.properties ->> 'status' AS target_status
FROM graph_results gr
JOIN default n1 ON n1.id = gr.source_id
JOIN default n2 ON n2.id = gr.target_id
WHERE n2.properties ->> 'status' = 'published';`,
        },
      ],
    },
    {
      id: 'ddl',
      title: 'Schema Management (DDL)',
      icon: <Database className="w-4 h-4" />,
      description: 'Create, alter, and drop NodeTypes, Archetypes, and ElementTypes using DDL syntax',
      examples: [
        {
          title: 'Create Basic NodeType',
          description: 'Create a simple node type with properties, labels, and ordering',
          sql: `-- Create a basic Page node type with UI hints
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
);`,
        },
        {
          title: 'Create NodeType with Nested Object',
          description: 'Define node types with nested object properties and metadata',
          sql: `-- Create a node type with deeply nested SEO object
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
);`,
        },
        {
          title: 'Create NodeType with Indexes',
          description: 'Create a node type with full-text and property indexes',
          sql: `-- Create a searchable Product node type
-- Use FULLTEXT modifier for full-text search
-- Use PROPERTY_INDEX for fast filtering
CREATE NODETYPE 'shop:Product'
PROPERTIES (
  name String REQUIRED FULLTEXT,
  description String FULLTEXT,
  sku String REQUIRED UNIQUE PROPERTY_INDEX,
  price Number REQUIRED PROPERTY_INDEX,
  category String PROPERTY_INDEX,
  tags Array OF String FULLTEXT,
  in_stock Boolean DEFAULT true
);

-- FULLTEXT enables FULLTEXT_MATCH() searches
-- PROPERTY_INDEX speeds up WHERE clause filtering`,
        },
        {
          title: 'Create NodeType that Extends Another',
          description: 'Inherit properties from a base node type',
          sql: `-- Create a base Content type
CREATE NODETYPE 'cms:Content'
PROPERTIES (
  title String REQUIRED FULLTEXT,
  slug String,
  status String DEFAULT 'draft',
  author String
)
VERSIONABLE
PUBLISHABLE;

-- Create Article that extends Content
CREATE NODETYPE 'cms:Article'
EXTENDS 'cms:Content'
PROPERTIES (
  body String FULLTEXT TRANSLATABLE,
  excerpt String,
  featured_image Resource,
  category String
);

-- Article inherits all Content properties plus its own`,
        },
        {
          title: 'Create NodeType with Allowed Children',
          description: 'Restrict which node types can be children',
          sql: `-- Create a Blog that can only contain Posts and Categories
CREATE NODETYPE 'cms:Blog'
PROPERTIES (
  name String REQUIRED,
  description String
)
ALLOWED_CHILDREN ('cms:Post', 'cms:Category');

-- Create a Category that can contain Posts
CREATE NODETYPE 'cms:Category'
PROPERTIES (
  name String REQUIRED,
  slug String UNIQUE
)
ALLOWED_CHILDREN ('cms:Post');

-- Create Post (leaf node - no children specified)
CREATE NODETYPE 'cms:Post'
PROPERTIES (
  title String REQUIRED FULLTEXT,
  body String FULLTEXT
)
PUBLISHABLE;`,
        },
        {
          title: 'Create NodeType with Flags',
          description: 'Use behavior flags for versioning, publishing, and auditing',
          sql: `-- All available flags for node types
CREATE NODETYPE 'cms:Document'
DESCRIPTION 'Versioned document type'
ICON 'document'
PROPERTIES (
  title String REQUIRED,
  content String FULLTEXT
)
VERSIONABLE   -- Enable version history
PUBLISHABLE   -- Enable publish workflow
AUDITABLE     -- Track all changes
INDEXABLE     -- Include in search indexes
STRICT;       -- Enforce strict property validation`,
        },
        {
          title: 'All Property Types',
          description: 'Comprehensive example with all supported property types',
          sql: `-- All supported property types in RaisinDB DDL
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
);`,
        },
        {
          title: 'Complex E-commerce Product',
          description: 'Real-world example with deep nesting and all features',
          sql: `-- Complex product type with deep nesting
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
);`,
        },
        {
          title: 'Create Archetype',
          description: 'Create an archetype (pre-configured node template)',
          sql: `-- Create a Blog Post archetype based on cms:Article
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
-- when creating new nodes of that type`,
        },
        {
          title: 'Create ElementType',
          description: 'Create an element type for reusable components',
          sql: `-- Create a Hero Banner element type
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

-- ElementTypes define reusable content blocks`,
        },
        {
          title: 'Alter NodeType - Add Property',
          description: 'Add a new property to an existing node type',
          sql: `-- Add a new property to an existing node type
ALTER NODETYPE 'cms:Article'
ADD PROPERTY summary String FULLTEXT;

-- Add a required property with default
ALTER NODETYPE 'cms:Article'
ADD PROPERTY language String REQUIRED DEFAULT 'en';

-- Add an object property
ALTER NODETYPE 'cms:Article'
ADD PROPERTY analytics Object {
  views Number DEFAULT 0,
  shares Number DEFAULT 0,
  comments Number DEFAULT 0
};`,
        },
        {
          title: 'Alter Nested Properties',
          description: 'Modify properties inside nested objects using dotted paths',
          sql: `-- For a NodeType with nested objects:
-- specs Object { dimensions Object { width Number, height Number } }

-- Modify a nested property (use quoted dotted path)
ALTER NODETYPE 'ecommerce:Product'
MODIFY PROPERTY 'specs.dimensions.width' Number LABEL 'Width (cm)';

-- Add a new property inside a nested object
ALTER NODETYPE 'ecommerce:Product'
ADD PROPERTY 'specs.dimensions.depth' Number LABEL 'Depth (cm)' DEFAULT 0;

-- Drop a nested property
ALTER NODETYPE 'ecommerce:Product'
DROP PROPERTY 'specs.dimensions.legacy_field';

-- Deeply nested (4+ levels)
ALTER NODETYPE 'cms:Page'
MODIFY PROPERTY 'seo.social.twitter.card_type' String DEFAULT 'summary'
ADD PROPERTY 'seo.social.twitter.creator' String LABEL 'Twitter Creator'
DROP PROPERTY 'seo.social.twitter.old_field';

-- Mix simple and nested alterations
ALTER NODETYPE 'cms:Article'
ADD PROPERTY excerpt String FULLTEXT
MODIFY PROPERTY 'seo.title' String LABEL 'SEO Title'
DROP PROPERTY legacy_field
DROP PROPERTY 'meta.old_setting';`,
        },
        {
          title: 'Alter NodeType - Other Changes',
          description: 'Modify or drop properties, change settings',
          sql: `-- Remove a property from an existing node type
ALTER NODETYPE 'cms:Article'
DROP PROPERTY legacy_field;

-- Change description
ALTER NODETYPE 'cms:Article'
SET DESCRIPTION = 'Updated article type';

-- Change icon
ALTER NODETYPE 'cms:Article'
SET ICON = 'newspaper';

-- Change parent type
ALTER NODETYPE 'cms:Article'
SET EXTENDS = 'cms:NewBase';

-- Update allowed children
ALTER NODETYPE 'cms:Blog'
SET ALLOWED_CHILDREN = ('cms:Post', 'cms:Category', 'cms:Page');

-- Toggle flags
ALTER NODETYPE 'cms:Article'
SET PUBLISHABLE = true;`,
        },
        {
          title: 'Drop Schema Objects',
          description: 'Remove node types, archetypes, or element types',
          sql: `-- Drop a node type
DROP NODETYPE 'cms:DeprecatedType';

-- Drop with CASCADE (removes dependent objects)
DROP NODETYPE 'cms:OldType' CASCADE;

-- Drop an archetype
DROP ARCHETYPE 'old-template';

-- Drop an element type
DROP ELEMENTTYPE 'ui:OldComponent';`,
        },
        {
          title: 'View Schema Tables',
          description: 'Query the schema tables to see defined types',
          sql: `-- View all node types
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
-- Use DDL (CREATE/ALTER/DROP) to modify schema`,
        },
      ],
    },
    {
      id: 'dml',
      title: 'Data Manipulation (DML)',
      icon: <PenLine className="w-4 h-4" />,
      description: 'Insert, update, and delete nodes using SQL with transaction support',
      examples: [
        {
          title: 'Insert a Node',
          description: 'Create a new node in a workspace using INSERT statement',
          sql: `-- Insert a new node into the workspace
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
);`,
        },
        {
          title: 'Insert Multiple Nodes',
          description: 'Insert multiple nodes in a single statement',
          sql: `-- Insert multiple nodes at once
INSERT INTO default (path, node_type, name)
VALUES
  ('/content/pages/about', 'raisin:Page', 'About Us'),
  ('/content/pages/contact', 'raisin:Page', 'Contact'),
  ('/content/pages/faq', 'raisin:Page', 'FAQ');`,
        },
        {
          title: 'Update Node Properties',
          description: 'Update an existing node by ID or path',
          sql: `-- Update by node ID (UUID format)
UPDATE default
SET properties = '{"status": "published", "views": 100}'
WHERE id = 'ccf1eaae-e33b-4915-92fd-87b5518ba30d';

-- Update by path
UPDATE default
SET properties = '{"title": "Updated Title"}'
WHERE path = '/content/blog/my-post';

-- Merge properties (add/update keys, keep existing)
UPDATE default
SET properties = properties || '{"featured": true, "priority": 1}'
WHERE id = 'ccf1eaae-e33b-4915-92fd-87b5518ba30d';

-- Note: WHERE clause must use 'id' or 'path' only
-- Complex WHERE clauses are not yet supported`,
        },
        {
          title: 'Delete a Node',
          description: 'Remove a node by ID or path',
          sql: `-- Delete by node ID
DELETE FROM default
WHERE id = 'abc123';

-- Delete by path
DELETE FROM default
WHERE path = '/content/blog/old-post';

-- Note: Deleting a parent does NOT cascade to children
-- Delete children first if needed`,
        },
        {
          title: 'Insert with JSON Properties',
          description: 'Create nodes with rich JSON property structures',
          sql: `-- Insert with flat JSON properties
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
);`,
        },
        {
          title: 'Update JSON Properties (Replace)',
          description: 'Replace entire properties object or specific top-level keys',
          sql: `-- Replace entire properties object
UPDATE default
SET properties = '{"status": "archived", "archivedAt": "2024-01-15"}'
WHERE path = '/content/old-page';

-- Set a single property using jsonb_set (PostgreSQL style)
-- Note: This replaces the key if exists, adds if not
UPDATE default
SET properties = jsonb_set(properties, '{status}', '"published"')
WHERE path = '/content/blog/post1';

-- Set nested property
UPDATE default
SET properties = jsonb_set(properties, '{seo,title}', '"New SEO Title"')
WHERE path = '/content/blog/post1';

-- Set deeply nested property
UPDATE default
SET properties = jsonb_set(
  properties,
  '{metadata,social,twitter}',
  '"@myhandle"'
)
WHERE path = '/content/blog/post1';`,
        },
        {
          title: 'Update JSON Properties (Merge)',
          description: 'Merge new properties with existing ones using || operator',
          sql: `-- Merge properties (add or update keys, keep others)
-- The || operator merges JSON objects
UPDATE default
SET properties = properties || '{"featured": true, "priority": 1}'
WHERE path = '/content/blog/post1';

-- Add new nested object while keeping existing properties
UPDATE default
SET properties = properties || '{
  "analytics": {
    "pageViews": 0,
    "uniqueVisitors": 0,
    "avgTimeOnPage": 0
  }
}'
WHERE node_type = 'raisin:Page'
  AND NOT (properties ? 'analytics');

-- Update multiple properties at once
UPDATE default
SET properties = properties || '{
  "status": "published",
  "publishedAt": "2024-01-15T10:30:00Z",
  "publishedBy": "editor@example.com"
}'
WHERE path = '/content/blog/post1';`,
        },
        {
          title: 'Update JSON Arrays',
          description: 'Modify array properties within JSON',
          sql: `-- Append to array using jsonb_set with concatenation
UPDATE default
SET properties = jsonb_set(
  properties,
  '{tags}',
  (properties -> 'tags') || '["new-tag"]'
)
WHERE path = '/content/blog/post1';

-- Replace entire array
UPDATE default
SET properties = jsonb_set(
  properties,
  '{categories}',
  '["tech", "tutorial", "rust"]'
)
WHERE path = '/content/blog/post1';

-- Set array element by index (0-based)
UPDATE default
SET properties = jsonb_set(
  properties,
  '{tags, 0}',
  '"first-tag"'
)
WHERE path = '/content/blog/post1';`,
        },
        {
          title: 'Remove JSON Properties',
          description: 'Delete specific keys from properties object',
          sql: `-- Remove a single property key using - operator
UPDATE default
SET properties = properties - 'deprecated_field'
WHERE path = '/content/blog/post1';

-- Remove multiple keys
UPDATE default
SET properties = properties - 'field1' - 'field2' - 'field3'
WHERE node_type = 'raisin:Page';

-- Remove nested key using #- operator with path
UPDATE default
SET properties = properties #- '{seo,oldField}'
WHERE path = '/content/blog/post1';

-- Remove deeply nested key
UPDATE default
SET properties = properties #- '{metadata,social,deprecated}'
WHERE path = '/content/blog/post1';`,
        },
        {
          title: 'Conditional JSON Updates',
          description: 'Update properties based on existing JSON values',
          sql: `-- Update only if property exists
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
  AND (properties ->> 'createdAt')::timestamp < '2024-01-01';`,
        },
        {
          title: 'Complex JSON Property Patterns',
          description: 'Advanced patterns for working with JSON properties',
          sql: `-- Initialize default properties structure on insert
INSERT INTO default (path, node_type, name, properties)
VALUES (
  '/content/blog/new-post',
  'raisin:Page',
  'New Post',
  '{
    "title": "New Post",
    "status": "draft",
    "author": null,
    "seo": {"title": null, "description": null},
    "metadata": {"views": 0, "likes": 0, "shares": 0},
    "tags": [],
    "relatedPosts": []
  }'
);

-- Copy properties structure from another node (via subquery pattern)
-- Note: Requires application logic, shown as concept
-- INSERT INTO default (path, node_type, name, properties)
-- SELECT '/content/blog/copy', node_type, 'Copy of Post', properties
-- FROM default WHERE path = '/content/blog/original';

-- Increment numeric property
UPDATE default
SET properties = jsonb_set(
  properties,
  '{metadata,views}',
  to_jsonb((properties -> 'metadata' ->> 'views')::int + 1)
)
WHERE path = '/content/blog/post1';

-- Toggle boolean property
UPDATE default
SET properties = jsonb_set(
  properties,
  '{featured}',
  to_jsonb(NOT (properties ->> 'featured')::boolean)
)
WHERE path = '/content/blog/post1';`,
        },
        {
          title: 'Basic Transaction',
          description: 'Group multiple DML operations into a single atomic commit',
          sql: `-- Start a transaction
BEGIN;

-- Multiple operations are buffered
INSERT INTO default (path, node_type, name)
VALUES ('/content/blog/post1', 'raisin:Page', 'First Post');

INSERT INTO default (path, node_type, name)
VALUES ('/content/blog/post2', 'raisin:Page', 'Second Post');

UPDATE default
SET properties = '{"status": "draft"}'
WHERE path = '/content/blog/post1';

-- Commit all changes atomically
COMMIT;`,
        },
        {
          title: 'Transaction with Message',
          description: 'Commit with a custom message for the revision history',
          sql: `-- Start transaction
BEGIN;

-- Make changes
INSERT INTO default (path, node_type, name)
VALUES ('/products/new-item', 'shop:Product', 'New Product');

UPDATE default
SET properties = '{"price": 29.99}'
WHERE path = '/products/new-item';

-- Commit with descriptive message (shows in revision history)
COMMIT WITH MESSAGE 'Added new product with pricing';`,
        },
        {
          title: 'Transaction with Message and Actor',
          description: 'Specify both commit message and actor (author) for audit trail',
          sql: `-- Start transaction
BEGIN;

-- Bulk update
UPDATE default
SET properties = properties || '{"reviewed": true}'
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND properties ->> 'status' = 'draft';

-- Commit with message and actor for audit trail
COMMIT WITH MESSAGE 'Marked blog posts as reviewed' ACTOR 'editor@example.com';

-- Actor defaults to 'system' if not specified
-- Message defaults to 'SQL transaction' if not specified`,
        },
        {
          title: 'Auto-Commit Mode',
          description: 'DML without BEGIN auto-commits each statement',
          sql: `-- Without BEGIN, each statement auto-commits immediately
-- This creates a separate revision for each operation

-- Auto-commits immediately
INSERT INTO default (path, node_type, name)
VALUES ('/content/page1', 'raisin:Page', 'Page 1');

-- This is a separate commit
INSERT INTO default (path, node_type, name)
VALUES ('/content/page2', 'raisin:Page', 'Page 2');

-- Use BEGIN/COMMIT to batch operations into single revision`,
        },
        {
          title: 'Create Folder Structure',
          description: 'Build a hierarchical folder structure with transaction',
          sql: `-- Create a folder structure atomically
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

COMMIT WITH MESSAGE 'Created blog folder structure';`,
        },
        {
          title: 'Bulk Content Migration',
          description: 'Example of migrating content with proper transaction handling',
          sql: `-- Migrate content to new structure
BEGIN;

-- Create new category folders
INSERT INTO default (path, node_type, name)
VALUES
  ('/content/articles', 'raisin:Folder', 'Articles'),
  ('/content/articles/tech', 'raisin:Folder', 'Technology'),
  ('/content/articles/news', 'raisin:Folder', 'News');

-- Update existing posts to new paths would require
-- application-level logic to move nodes

COMMIT WITH MESSAGE 'Set up new content structure' ACTOR 'migration-script';`,
        },
        {
          title: 'ORDER with ABOVE',
          description: 'Reorder sibling nodes - move a node before another',
          sql: `-- ORDER statement reorders children within a parent
-- Syntax: ORDER <workspace> [IN BRANCH 'name'] SET path='<node>' ABOVE path='<sibling>'
-- ABOVE moves the source node to appear BEFORE the target

-- Before: [alice, bob, carol]
ORDER default SET path='/users/carol' ABOVE path='/users/bob';
-- Result: [alice, carol, bob]

-- Move to first position (above the first child)
ORDER default SET path='/users/carol' ABOVE path='/users/alice';
-- Result: [carol, alice, bob]

-- Works with nested paths too
ORDER default SET path='/content/blog/post3' ABOVE path='/content/blog/post1';

-- Order on a specific branch
ORDER default IN BRANCH 'feature' SET path='/items/c' ABOVE path='/items/a';

-- Note: Both nodes must be siblings (same parent)
-- Self-ordering or already-in-position is a no-op`,
        },
        {
          title: 'ORDER with BELOW',
          description: 'Reorder sibling nodes - move a node after another',
          sql: `-- ORDER statement reorders children within a parent
-- Syntax: ORDER <workspace> [IN BRANCH 'name'] SET path='<node>' BELOW path='<sibling>'
-- BELOW moves the source node to appear AFTER the target

-- Before: [alice, bob, carol]
ORDER default SET path='/users/alice' BELOW path='/users/carol';
-- Result: [bob, carol, alice]

-- Move to last position (below the last child)
ORDER default SET path='/users/bob' BELOW path='/users/carol';
-- Result: [alice, carol, bob]

-- Order on a specific branch
ORDER default IN BRANCH 'staging' SET path='/items/b' BELOW path='/items/a';

-- Chain multiple ORDER operations for complex reordering
ORDER default SET path='/items/c' BELOW path='/items/a';
ORDER default SET path='/items/b' ABOVE path='/items/c';
-- Result: [a, b, c]`,
        },
        {
          title: 'MOVE - Reparent Node Subtree',
          description: 'Move a node and its entire subtree to a new parent location',
          sql: `-- MOVE statement moves a node (and all descendants) to a new parent
-- Syntax: MOVE <workspace> [IN BRANCH 'name'] SET path='<source>' TO path='<new-parent>'
-- Note: Node IDs are preserved during move operations

-- Move a blog post to a different category
MOVE default SET path='/content/blog/draft-post' TO path='/content/blog/published';
-- Result: /content/blog/draft-post → /content/blog/published/draft-post

-- Move an entire folder with all its contents
MOVE default SET path='/content/old-section' TO path='/content/archive';
-- Result: All descendants move with their parent

-- Move on a specific branch
MOVE default IN BRANCH 'feature' SET path='/content/draft' TO path='/content/ready';

-- Move by node ID instead of path
MOVE default SET id='abc123' TO path='/content/new-parent';

-- Move to root level (parent = /)
MOVE default SET path='/content/orphan' TO path='/';
-- Result: /orphan becomes a root-level node`,
        },
        {
          title: 'MOVE - Reorganize Content',
          description: 'Use MOVE to restructure content hierarchy while preserving node IDs',
          sql: `-- MOVE preserves node IDs - important for:
-- - External references to nodes
-- - Maintaining relation integrity
-- - Preserving history and audit trails

-- Reorganize blog posts into year-based folders
MOVE default SET path='/content/blog/old-post' TO path='/content/blog/2024';

-- Move entire year folder under archive
MOVE default SET path='/content/blog/2023' TO path='/content/archive/blog';

-- Move multiple nodes (execute in sequence)
MOVE default SET path='/users/alice' TO path='/org/engineering';
MOVE default SET path='/users/bob' TO path='/org/engineering';
MOVE default SET path='/users/carol' TO path='/org/marketing';

-- Note: Unlike DELETE + INSERT, MOVE keeps all node IDs intact`,
        },
        {
          title: 'COPY - Duplicate Single Node',
          description: 'Copy a single node to a new parent location with a new ID',
          sql: `-- COPY statement duplicates a node to a new location
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
-- For preserving IDs, use MOVE instead`,
        },
        {
          title: 'COPY TREE - Duplicate Entire Subtree',
          description: 'Copy a node and all its descendants to a new parent location',
          sql: `-- COPY TREE statement duplicates a node AND all descendants
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

-- Note: For large trees (>5000 nodes), operation may run as background job`,
        },
        {
          title: 'COPY vs MOVE Comparison',
          description: 'When to use COPY versus MOVE for content operations',
          sql: `-- MOVE: Reparent nodes (keeps IDs, changes paths)
-- ✓ External references remain valid
-- ✓ History and audit trails preserved
-- ✓ Best for reorganizing content
MOVE default SET path='/blog/draft-post' TO path='/blog/published';
-- Node ID stays the same, path changes

-- COPY: Duplicate nodes (new IDs, new paths)
-- ✓ Creates independent copies
-- ✓ Original content unaffected
-- ✓ Best for templates and content duplication
COPY default SET path='/templates/post' TO path='/blog' AS 'new-post';
-- New node with new ID is created

-- COPY TREE: Duplicate entire subtrees
-- ✓ Copies node AND all descendants
-- ✓ All copied nodes get new IDs
-- ✓ Best for duplicating entire sections
COPY TREE default SET path='/templates/section' TO path='/content';
-- Creates copy of entire subtree with new IDs`,
        },
        {
          title: 'Branch-Specific Operations (IN BRANCH)',
          description: 'Execute ORDER, MOVE, COPY, and TRANSLATE on a specific branch instead of the default',
          sql: `-- ORDER, MOVE, COPY, and TRANSLATE support optional IN BRANCH clause
-- Syntax: <statement> IN BRANCH 'branch-name' ...

-- ORDER on a specific branch
ORDER default IN BRANCH 'feature-branch' SET path='/content/post3' ABOVE path='/content/post1';

-- MOVE on a specific branch
MOVE default IN BRANCH 'staging' SET path='/content/draft' TO path='/content/published';

-- COPY on a specific branch
COPY TREE default IN BRANCH 'feature' SET path='/templates/page' TO path='/content' AS 'new-page';

-- TRANSLATE on a specific branch
TRANSLATE Article IN BRANCH 'preview' FOR LOCALE 'de' SET /title = 'Titel' WHERE path = '/content/article';

-- Without IN BRANCH, operations use the default branch from context
ORDER default SET path='/items/b' ABOVE path='/items/a';
MOVE default SET path='/old/path' TO path='/new/parent';
COPY default SET path='/templates/page' TO path='/content';
TRANSLATE Article FOR LOCALE 'fr' SET /title = 'Titre' WHERE path = '/content/article';`,
        },
        {
          title: 'Branch-Specific UPDATE/DELETE (__branch)',
          description: 'Execute UPDATE and DELETE operations on a specific branch using __branch in WHERE clause',
          sql: `-- For UPDATE and DELETE, use __branch = 'name' in WHERE clause
-- The __branch predicate is extracted and used for branch selection

-- UPDATE on a specific branch
UPDATE default
SET properties = '{"status": "published"}'
WHERE path = '/content/article'
  AND __branch = 'feature-branch';

-- DELETE on a specific branch
DELETE FROM default
WHERE id = 'abc123'
  AND __branch = 'staging';

-- Combine with other conditions
UPDATE default
SET properties = properties || '{"reviewed": true}'
WHERE path = '/content/blog/post1'
  AND __branch = 'preview'
  AND properties ->> 'status' = 'draft';

-- Without __branch, operations use the default branch from context
UPDATE default SET properties = '{}' WHERE path = '/content/page';
DELETE FROM default WHERE id = 'xyz789';`,
        },
      ],
    },
    {
      id: 'relations',
      title: 'Node Relations',
      icon: <Network className="w-4 h-4" />,
      description: 'Create and manage directed relationships between nodes with RELATE and UNRELATE',
      examples: [
        {
          title: 'RELATE - Create Basic Relation',
          description: 'Create a directed relationship from one node to another',
          sql: `-- RELATE creates a directed relationship between two nodes
-- Syntax: RELATE FROM <source> TO <target> [TYPE 'relation_type'] [WEIGHT number]
-- Node references can use: path='/path' or id='uuid'

-- Create a basic relation (defaults to type 'references')
RELATE FROM path='/content/blog/post1' TO path='/content/blog/post2';

-- Create a relation by node ID
RELATE FROM id='abc123' TO id='def456';

-- Mix path and id references
RELATE FROM path='/content/blog/post1' TO id='def456';`,
        },
        {
          title: 'RELATE - With Relation Type',
          description: 'Create typed relationships for semantic connections',
          sql: `-- Specify a custom relation type
RELATE FROM path='/content/blog/post1' TO path='/content/blog/post2' TYPE 'related-to';

-- Common relation type patterns:
RELATE FROM path='/content/articles/a1' TO path='/content/articles/a2' TYPE 'see-also';
RELATE FROM path='/products/laptop' TO path='/products/case' TYPE 'accessory';
RELATE FROM path='/users/alice' TO path='/users/bob' TYPE 'follows';
RELATE FROM path='/docs/chapter1' TO path='/docs/chapter2' TYPE 'next';`,
        },
        {
          title: 'RELATE - With Weight',
          description: 'Add numeric weight to relationships for ranking or scoring',
          sql: `-- Add weight for ranked/scored relationships
RELATE FROM path='/content/post1' TO path='/content/post2' WEIGHT 0.8;

-- Combine type and weight
RELATE FROM path='/content/article1' TO path='/content/article2'
  TYPE 'similarity'
  WEIGHT 0.95;

-- Weight is useful for:
-- - Ranking recommendations (higher weight = stronger recommendation)
-- - Content similarity scores
-- - Relationship strength`,
        },
        {
          title: 'RELATE - Cross-Workspace',
          description: 'Create relationships between nodes in different workspaces',
          sql: `-- Relate nodes across workspaces using IN WORKSPACE clause
RELATE
  FROM path='/content/blog/post1' IN WORKSPACE 'website'
  TO path='/products/item1' IN WORKSPACE 'ecommerce';

-- Mix workspace specifications
RELATE
  FROM path='/articles/a1'  -- Uses default workspace
  TO path='/products/p1' IN WORKSPACE 'shop'
  TYPE 'mentions';

-- Workspace defaults to 'default' if not specified
-- Useful for connecting content across different sites/apps`,
        },
        {
          title: 'RELATE - Branch Override',
          description: 'Create relationships on a specific branch instead of default',
          sql: `-- Create relation on a specific branch
RELATE IN BRANCH 'feature-branch'
  FROM path='/content/draft1'
  TO path='/content/draft2'
  TYPE 'related';

-- Full syntax with all options
RELATE IN BRANCH 'preview'
  FROM path='/blog/post1' IN WORKSPACE 'content'
  TO path='/blog/post2' IN WORKSPACE 'content'
  TYPE 'see-also'
  WEIGHT 0.75;`,
        },
        {
          title: 'UNRELATE - Remove Basic Relation',
          description: 'Remove a directed relationship between nodes',
          sql: `-- UNRELATE removes a relationship between two nodes
-- Syntax: UNRELATE FROM <source> TO <target> [TYPE 'relation_type']

-- Remove relation (removes all types between source and target)
UNRELATE FROM path='/content/blog/post1' TO path='/content/blog/post2';

-- Remove by node ID
UNRELATE FROM id='abc123' TO id='def456';`,
        },
        {
          title: 'UNRELATE - With Relation Type',
          description: 'Remove only a specific type of relationship',
          sql: `-- Remove only a specific relation type
UNRELATE FROM path='/content/post1' TO path='/content/post2' TYPE 'related-to';

-- Remove specific types while keeping others
UNRELATE FROM path='/users/alice' TO path='/users/bob' TYPE 'follows';

-- If TYPE is not specified, ALL relations between the nodes are removed`,
        },
        {
          title: 'UNRELATE - Cross-Workspace',
          description: 'Remove relationships between nodes in different workspaces',
          sql: `-- Remove cross-workspace relations
UNRELATE
  FROM path='/content/blog/post1' IN WORKSPACE 'website'
  TO path='/products/item1' IN WORKSPACE 'ecommerce';

-- With specific type
UNRELATE
  FROM path='/articles/a1' IN WORKSPACE 'content'
  TO path='/products/p1' IN WORKSPACE 'shop'
  TYPE 'mentions';`,
        },
        {
          title: 'UNRELATE - Branch Override',
          description: 'Remove relationships on a specific branch',
          sql: `-- Remove relation on a specific branch
UNRELATE IN BRANCH 'feature-branch'
  FROM path='/content/draft1'
  TO path='/content/draft2';

-- Full syntax with all options
UNRELATE IN BRANCH 'preview'
  FROM path='/blog/post1' IN WORKSPACE 'content'
  TO path='/blog/post2' IN WORKSPACE 'content'
  TYPE 'see-also';`,
        },
        {
          title: 'Query Relations with CYPHER()',
          description: 'Use Cypher graph queries to traverse relationships created with RELATE',
          sql: `-- CYPHER() queries the graph layer built by RELATE statements
-- Relationships appear as edges with their TYPE as the relationship type

-- Find all outgoing relations from a node
SELECT * FROM CYPHER('
  MATCH (a)-[r]->(b)
  WHERE a.path = "/content/blog/post1"
  RETURN a.name AS source, type(r) AS relation, b.name AS target
');

-- Follow specific relation types (created via RELATE ... TYPE 'see-also')
SELECT * FROM CYPHER('
  MATCH (a)-[:see-also]->(b)
  WHERE a.path STARTS WITH "/content"
  RETURN a.path AS from_path, b.path AS to_path
');

-- Find nodes with weighted relations (WEIGHT becomes edge property)
SELECT * FROM CYPHER('
  MATCH (a)-[r:similarity]->(b)
  WHERE r.weight > 0.8
  RETURN a.name, b.name, r.weight
  ORDER BY r.weight DESC
');`,
        },
        {
          title: 'JOIN Graph Results with Nodes',
          description: 'Combine Cypher graph traversal with SQL to get full node data',
          sql: `-- Use WITH clause to join graph results with node table
WITH related AS (
  SELECT * FROM CYPHER('
    MATCH (a)-[:related-to]->(b)
    WHERE a.path = "/content/blog/post1"
    RETURN b.id AS related_id, b.path AS related_path
  ')
)
SELECT
  n.id,
  n.name,
  n.path,
  n.node_type,
  n.properties ->> 'title' AS title,
  n.properties ->> 'status' AS status
FROM default n
JOIN related r ON n.id = r.related_id
WHERE n.properties ->> 'status' = 'published';

-- Get recommendations with similarity scores
WITH recommendations AS (
  SELECT * FROM CYPHER('
    MATCH (source)-[r:similarity]->(target)
    WHERE source.path = "/products/laptop"
    RETURN target.id AS product_id, r.weight AS score
    ORDER BY r.weight DESC
    LIMIT 10
  ')
)
SELECT
  p.id,
  p.name,
  p.path,
  p.properties ->> 'price' AS price,
  rec.score AS similarity_score
FROM default p
JOIN recommendations rec ON p.id = rec.product_id
ORDER BY rec.score DESC;`,
        },
        {
          title: 'Graph Traversal Patterns',
          description: 'Common patterns for traversing relation graphs',
          sql: `-- Multi-hop traversal (find related content 2 hops away)
SELECT * FROM CYPHER('
  MATCH (a)-[:related-to*1..2]->(b)
  WHERE a.path = "/content/article1"
  RETURN DISTINCT b.path AS related_path, length(path) AS hops
');

-- Find all content that follows a specific item
SELECT * FROM CYPHER('
  MATCH (follower)-[:follows]->(target)
  WHERE target.path = "/users/popular-author"
  RETURN follower.name, follower.path
');

-- Bidirectional relationship check
SELECT * FROM CYPHER('
  MATCH (a)-[:follows]->(b), (b)-[:follows]->(a)
  RETURN a.name AS user1, b.name AS user2
');

-- Path between nodes
SELECT * FROM CYPHER('
  MATCH path = shortestPath((a)-[*]-(b))
  WHERE a.path = "/content/start" AND b.path = "/content/end"
  RETURN [n IN nodes(path) | n.name] AS path_names
');`,
        },
        {
          title: 'lookup() - Fetch Full Node in Cypher',
          description: 'Use lookup(id, workspace) to fetch complete node data within Cypher queries',
          sql: `-- lookup(id, workspace) fetches a node by ID and workspace
-- Returns object with: id, workspace, path, type, properties

-- Get full node data for related nodes
SELECT * FROM CYPHER('
  MATCH (a)-[:related-to]->(b)
  WHERE a.path = "/content/blog/post1"
  RETURN lookup(b.id, b.workspace) AS related_node
');

-- Access properties from looked-up node
SELECT * FROM CYPHER('
  MATCH (a)-[:follows]->(b)
  WHERE a.path = "/users/alice"
  WITH b, lookup(b.id, b.workspace) AS full_node
  RETURN b.name, full_node.properties.bio AS bio, full_node.properties.avatar AS avatar
');

-- Combine with filtering on fetched properties
SELECT * FROM CYPHER('
  MATCH (source)-[:recommends]->(target)
  WHERE source.path = "/products/laptop"
  WITH target, lookup(target.id, target.workspace) AS product
  WHERE product.properties.in_stock = true
  RETURN product.path, product.properties.name, product.properties.price
');`,
        },
        {
          title: 'Cypher Graph Functions',
          description: 'Built-in functions for graph analysis within CYPHER queries',
          sql: `-- DEGREE FUNCTIONS
-- degree(node) - Total relationships (in + out)
-- inDegree(node) - Incoming relationships only
-- outDegree(node) - Outgoing relationships only
SELECT * FROM CYPHER('
  MATCH (n)
  WHERE n.path STARTS WITH "/users"
  RETURN n.name, degree(n) AS connections,
         inDegree(n) AS followers, outDegree(n) AS following
  ORDER BY degree(n) DESC
  LIMIT 10
');

-- CENTRALITY FUNCTIONS
-- pageRank(node) - PageRank score (importance)
-- closeness(node) - Closeness centrality
-- betweenness(node) - Betweenness centrality
SELECT * FROM CYPHER('
  MATCH (n)
  RETURN n.name, n.path,
         pageRank(n) AS importance,
         closeness(n) AS reachability
  ORDER BY pageRank(n) DESC
  LIMIT 20
');

-- COMMUNITY DETECTION
-- componentId(node) - Connected component ID
-- componentCount() - Number of components
-- communityId(node) - Detected community ID
-- communityCount() - Number of communities
SELECT * FROM CYPHER('
  MATCH (n)
  RETURN n.name, communityId(n) AS community
  ORDER BY community
');

-- PATH FINDING
-- shortestPath(start, end) - Find shortest path
-- allShortestPaths(start, end) - All shortest paths
-- distance(start, end) - Path length
SELECT * FROM CYPHER('
  MATCH (a), (b)
  WHERE a.path = "/start" AND b.path = "/end"
  RETURN distance(a, b) AS hops
');`,
        },
        {
          title: 'Cypher Limitations & Workarounds',
          description: 'Known limitations and the SQL CTE workaround pattern',
          sql: `-- CYPHER LIMITATIONS
-- The following clauses are PARSED but NOT EXECUTED:
-- - WITH (use SQL CTEs instead)
-- - SET, DELETE, MERGE, UNWIND, REMOVE (not implemented)
--
-- SUPPORTED: MATCH, WHERE, RETURN (with ORDER BY, LIMIT, SKIP)

-- THIS DOES NOT WORK (WITH clause not implemented)
-- SELECT * FROM CYPHER('
--   MATCH (a)-[:relates]->(b)
--   WITH b, lookup(b.id, b.workspace) AS full
--   RETURN full.properties.title
-- ');

-- USE SQL CTE + JOIN INSTEAD
-- Pattern: CYPHER returns IDs → SQL joins with node table
WITH related AS (
  SELECT * FROM CYPHER('
    MATCH (corrector)-[:corrects]->(article)
    WHERE article.path = "/content/articles/my-article"
    RETURN corrector.id AS node_id, corrector.path AS node_path
    LIMIT 10
  ')
)
SELECT
  r.node_path,
  n.properties ->> 'title' AS title,
  n.properties ->> 'status' AS status
FROM related r
JOIN default n ON n.id = r.node_id;

-- Multi-hop with full node data
WITH recommendations AS (
  SELECT * FROM CYPHER('
    MATCH (a)-[:similar*1..2]->(b)
    WHERE a.path = "/products/laptop"
    RETURN DISTINCT b.id AS product_id
    LIMIT 20
  ')
)
SELECT
  p.path,
  p.properties ->> 'name' AS name,
  p.properties ->> 'price' AS price
FROM recommendations r
JOIN default p ON p.id = r.product_id;`,
        },
        {
          title: 'Relation Syntax Reference',
          description: 'Complete syntax reference for RELATE and UNRELATE statements',
          sql: `-- RELATE SYNTAX
-- RELATE [IN BRANCH 'branch']
--   FROM path|id='value' [IN WORKSPACE 'ws']
--   TO path|id='value' [IN WORKSPACE 'ws']
--   [TYPE 'relation_type']
--   [WEIGHT number];

-- Examples:
RELATE FROM path='/a' TO path='/b';                    -- Basic
RELATE FROM id='uuid1' TO id='uuid2';                  -- By ID
RELATE FROM path='/a' TO path='/b' TYPE 'related';     -- With type
RELATE FROM path='/a' TO path='/b' WEIGHT 0.5;         -- With weight
RELATE IN BRANCH 'dev' FROM path='/a' TO path='/b';    -- Branch
RELATE FROM path='/a' IN WORKSPACE 'ws1' TO path='/b'; -- Workspace

-- UNRELATE SYNTAX
-- UNRELATE [IN BRANCH 'branch']
--   FROM path|id='value' [IN WORKSPACE 'ws']
--   TO path|id='value' [IN WORKSPACE 'ws']
--   [TYPE 'relation_type'];

-- Examples:
UNRELATE FROM path='/a' TO path='/b';                  -- Remove all types
UNRELATE FROM path='/a' TO path='/b' TYPE 'related';   -- Specific type
UNRELATE IN BRANCH 'dev' FROM path='/a' TO path='/b';  -- Branch`,
        },
      ],
    },
    {
      id: 'datetime',
      title: 'Date, Time & Arithmetic',
      icon: <Clock className="w-4 h-4" />,
      description: 'PostgreSQL-compatible date/time functions, intervals, and arithmetic operations',
      examples: [
        {
          title: 'NOW() Function',
          description: 'Get the current timestamp with timezone (PostgreSQL-compatible)',
          sql: `-- NOW() returns current timestamp in UTC (TIMESTAMPTZ type)
SELECT
  id,
  name,
  path,
  created_at,
  NOW() AS current_time
FROM default
LIMIT 5;

-- Use in WHERE clause
SELECT id, name, created_at
FROM default
WHERE created_at < NOW()
ORDER BY created_at DESC;`,
        },
        {
          title: 'INTERVAL Syntax',
          description: 'PostgreSQL-style intervals for date/time arithmetic',
          sql: `-- INTERVAL supports multiple time units (PostgreSQL-compatible):
-- 'N hours', 'N days', 'N weeks', 'N months', 'N years'

-- Get content from the last 24 hours
SELECT id, name, path, created_at
FROM default
WHERE created_at >= NOW() - INTERVAL '24 hours'
ORDER BY created_at DESC;

-- Get content from the last 7 days
SELECT id, name, path, created_at
FROM default
WHERE created_at >= NOW() - INTERVAL '7 days'
ORDER BY created_at DESC;

-- Get content from the last month
SELECT id, name, path, created_at
FROM default
WHERE created_at >= NOW() - INTERVAL '1 month'
ORDER BY created_at DESC;`,
        },
        {
          title: 'Date Range Queries',
          description: 'Filter content by date ranges using intervals',
          sql: `-- Content updated in the last week
SELECT id, name, updated_at
FROM default
WHERE updated_at >= NOW() - INTERVAL '1 week'
ORDER BY updated_at DESC;

-- Content created between 1 and 2 weeks ago
SELECT id, name, created_at
FROM default
WHERE created_at >= NOW() - INTERVAL '2 weeks'
  AND created_at < NOW() - INTERVAL '1 week'
ORDER BY created_at;

-- Content older than 1 year
SELECT id, name, path, created_at
FROM default
WHERE created_at < NOW() - INTERVAL '1 year'
ORDER BY created_at;`,
        },
        {
          title: 'Timestamp Arithmetic',
          description: 'Add and subtract intervals from timestamps',
          sql: `-- PostgreSQL-compatible timestamp arithmetic:
-- TIMESTAMPTZ + INTERVAL → TIMESTAMPTZ
-- TIMESTAMPTZ - INTERVAL → TIMESTAMPTZ
-- TIMESTAMPTZ - TIMESTAMPTZ → INTERVAL

-- Calculate future dates
SELECT
  id,
  name,
  created_at,
  created_at + INTERVAL '30 days' AS expires_at,
  created_at + INTERVAL '1 year' AS anniversary
FROM default
LIMIT 10;

-- Calculate time since creation
SELECT
  id,
  name,
  created_at,
  NOW() - created_at AS age
FROM default
ORDER BY age DESC
LIMIT 10;`,
        },
        {
          title: 'Numeric Arithmetic',
          description: 'Standard arithmetic operators with automatic type promotion',
          sql: `-- ARITHMETIC OPERATORS (PostgreSQL-compatible):
-- +  Addition
-- -  Subtraction
-- *  Multiplication
-- /  Division
-- %  Modulo

-- Type promotion: INT → BIGINT → DOUBLE
-- Operations follow numeric ladder automatically

-- Calculate derived values
SELECT
  id,
  name,
  JSON_GET_DOUBLE(properties, 'price') AS price,
  JSON_GET_INT(properties, 'quantity') AS quantity,
  JSON_GET_DOUBLE(properties, 'price') * JSON_GET_INT(properties, 'quantity') AS total,
  JSON_GET_DOUBLE(properties, 'price') * 0.9 AS discounted_price
FROM default
WHERE properties ? 'price'
LIMIT 10;

-- Arithmetic with aggregates
SELECT
  PARENT(path) AS category,
  COUNT(*) AS items,
  SUM(JSON_GET_DOUBLE(properties, 'price')) AS total_value,
  AVG(JSON_GET_DOUBLE(properties, 'price')) AS avg_price,
  MAX(JSON_GET_DOUBLE(properties, 'price')) - MIN(JSON_GET_DOUBLE(properties, 'price')) AS price_range
FROM default
WHERE properties ? 'price'
GROUP BY PARENT(path);`,
        },
        {
          title: 'INTERVAL Units Reference',
          description: 'All supported interval units and syntax variations',
          sql: `-- INTERVAL UNITS (PostgreSQL-compatible):
-- All units support singular and plural forms

-- Time units:
-- INTERVAL 'N hours'    or INTERVAL 'N hour'
-- INTERVAL 'N minutes'  or INTERVAL 'N minute'
-- INTERVAL 'N seconds'  or INTERVAL 'N second'

-- Date units:
-- INTERVAL 'N days'     or INTERVAL 'N day'
-- INTERVAL 'N weeks'    or INTERVAL 'N week'
-- INTERVAL 'N months'   or INTERVAL 'N month'
-- INTERVAL 'N years'    or INTERVAL 'N year'

-- Examples:
SELECT
  NOW() AS now,
  NOW() - INTERVAL '1 hour' AS one_hour_ago,
  NOW() - INTERVAL '24 hours' AS yesterday,
  NOW() - INTERVAL '7 days' AS last_week,
  NOW() - INTERVAL '1 month' AS last_month,
  NOW() - INTERVAL '1 year' AS last_year;

-- Combine with WHERE for common queries
SELECT id, name, created_at
FROM default
WHERE created_at >= NOW() - INTERVAL '24 hours'
  AND node_type = 'raisin:Page';`,
        },
        {
          title: 'Comparison Operators',
          description: 'Standard SQL comparison operators for all types',
          sql: `-- COMPARISON OPERATORS (PostgreSQL-compatible):
-- =   Equal
-- <>  Not equal (also !=)
-- <   Less than
-- >   Greater than
-- <=  Less than or equal
-- >=  Greater than or equal

-- Works with all comparable types:
-- Numbers, Text, Timestamps, Paths

-- Numeric comparison
SELECT id, name
FROM default
WHERE JSON_GET_INT(properties, 'views') >= 1000;

-- Timestamp comparison
SELECT id, name, created_at
FROM default
WHERE created_at >= NOW() - INTERVAL '7 days'
  AND updated_at > created_at;

-- Text comparison (lexicographic)
SELECT id, name, path
FROM default
WHERE name >= 'A' AND name < 'B'
ORDER BY name;`,
        },
      ],
    },
    {
      id: 'reference',
      title: 'Quick Reference',
      icon: <BookOpen className="w-4 h-4" />,
      description: 'Operators, functions, and data types reference',
      examples: [
        {
          title: 'Hierarchy Functions',
          description: 'Path and hierarchy manipulation functions',
          sql: `-- HIERARCHY FUNCTIONS
-- PATH_STARTS_WITH(path, prefix) → Boolean
--   Check if path starts with prefix (efficient for descendants)

-- PARENT(path) → Path?
-- PARENT(path, levels) → Path?
--   Get parent path (NULL for root nodes)
--   Optional levels parameter: go N levels up (default: 1)
--   Examples:
--     PARENT('/a/b/c/d')     → '/a/b/c'  (immediate parent)
--     PARENT('/a/b/c/d', 2)  → '/a/b'    (grandparent)
--     PARENT('/a/b/c/d', 3)  → '/a'      (great-grandparent)

-- ANCESTOR(path, depth) → Path
--   Get ancestor at specific absolute depth from root
--   Returns empty string if depth exceeds path depth
--   Examples:
--     ANCESTOR('/a/b/c/d', 1)  → '/a'      (root level)
--     ANCESTOR('/a/b/c/d', 2)  → '/a/b'    (depth 2)
--     ANCESTOR('/a/b/c/d', 3)  → '/a/b/c'  (depth 3)

-- DEPTH(path) → Int
--   Get depth level (number of path segments)

-- DESCENDANT_OF(parent_path) → Boolean
-- DESCENDANT_OF(parent_path, max_depth) → Boolean
--   Check if current row's path is a descendant of parent_path
--   Optional max_depth limits how many levels deep:
--     DESCENDANT_OF('/a')     → all descendants (unlimited)
--     DESCENDANT_OF('/a', 1)  → direct children only
--     DESCENDANT_OF('/a', 2)  → children + grandchildren

-- Examples:
SELECT
  path,
  PARENT(path) AS parent,
  PARENT(path, 2) AS grandparent,
  ANCESTOR(path, 2) AS depth_2_ancestor,
  DEPTH(path) AS depth,
  CASE
    WHEN PATH_STARTS_WITH(path, '/content/') THEN 'content'
    ELSE 'other'
  END AS category
FROM default
LIMIT 10;`,
        },
        {
          title: 'JSON Operators & Functions',
          description: 'PostgreSQL-compatible JSON operators and extraction functions',
          sql: `-- JSON OPERATORS
-- ->   Extract JSON object/array (returns JSONB)
-- ->>  Extract as TEXT
-- @>   Contains (does JSON contain value?)
-- ?    Key exists
-- ?|   Any keys exist
-- ?&   All keys exist

-- JSON FUNCTIONS (Standard SQL:2016)
-- JSON_VALUE(jsonb, path) → Text?
--   Extract scalar value using JSONPath ($.nested.path)
-- JSON_EXISTS(jsonb, path) → Boolean
--   Check if JSONPath exists in document
--
-- JSON_GET_TEXT(jsonb, key) → Text?
--   Extract top-level key as text
-- JSON_GET_DOUBLE(jsonb, key) → Double?
--   Extract top-level key as double
-- JSON_GET_INT(jsonb, key) → Int?
--   Extract top-level key as integer
-- JSON_GET_BOOL(jsonb, key) → Boolean?
--   Extract top-level key as boolean

-- Operator examples:
SELECT
  properties -> 'metadata' AS metadata_obj,
  properties ->> 'title' AS title_text,
  properties @> '{"status": "published"}' AS is_published,
  properties ? 'tags' AS has_tags
FROM default
WHERE properties ->> 'author' = 'john'
LIMIT 5;

-- Function examples:
SELECT
  JSON_VALUE(properties, '$.seo.title') AS seo_title,
  JSON_EXISTS(properties, '$.metadata.social') AS has_social,
  JSON_GET_TEXT(properties, 'author') AS author,
  JSON_GET_DOUBLE(properties, 'price') AS price,
  JSON_GET_INT(properties, 'views') AS views,
  JSON_GET_BOOL(properties, 'featured') AS featured
FROM default
LIMIT 5;`,
        },
        {
          title: 'Full-Text Search Function',
          description: 'Tantivy-based full-text search',
          sql: `-- FULLTEXT_MATCH(query, language) → Boolean
--   Search pre-built Tantivy indexes
--   Only properties in node type's "properties_to_index" are searchable
--   Revision-aware: Respects max_revision from ExecutionContext
--   Workspace-scoped: Uses workspace from FROM clause table

-- Query operators (Tantivy syntax):
-- 'rust AND web'          -- Both terms required
-- 'rust OR python'        -- Either term
-- 'rust NOT javascript'   -- Exclude term
-- 'perform*'              -- Prefix wildcard
-- 'performnce~2'          -- Fuzzy match (edit distance 2)
-- '"high performance"'    -- Exact phrase

-- Supported languages:
-- 'english', 'german', 'french', 'spanish', 'simple'

-- Workspace scoping:
-- FULLTEXT_MATCH always uses the workspace from the FROM clause
-- Example: FROM default → searches default workspace
-- Example: FROM my_workspace → searches 'my_workspace' only
-- For cross-workspace search, use FULLTEXT_SEARCH table function

-- Revision control:
-- Default: Searches latest/HEAD documents
-- Override: Set max_revision in API request for point-in-time search

-- Complete example:
SELECT
  id,
  name,
  path,
  node_type,
  version,
  properties ->> 'title' AS title,
  properties ->> 'body' AS body_preview
FROM default
WHERE FULLTEXT_MATCH('(database OR storage) AND performance', 'english')
  AND properties ->> 'status' = 'published'
ORDER BY updated_at DESC
LIMIT 20;

-- Note: Set up indexing in schema.json first!
-- { "properties": { "body": { "fulltext": true } } }`,
        },
        {
          title: 'Vector Functions & Operators',
          description: 'Vector similarity search operations',
          sql: `-- VECTOR FUNCTIONS
-- EMBEDDING(text) → Vector
--   Generate embedding vector from text

-- VECTOR DISTANCE OPERATORS:
-- <->  L2 distance (Euclidean)
-- <=>  Cosine distance
-- <#>  Inner product

-- Examples:
SELECT
  id,
  name,
  path,
  node_type,
  properties ->> 'title' AS title,
  -- L2 distance (Euclidean)
  properties -> 'embedding' <-> EMBEDDING('search term') AS l2_dist,
  -- Cosine similarity (lower = more similar)
  properties -> 'embedding' <=> EMBEDDING('search term') AS cos_dist
FROM default
WHERE properties ? 'embedding'
ORDER BY l2_dist
LIMIT 10;`,
        },
        {
          title: 'Aggregate Functions',
          description: 'Available aggregation functions',
          sql: `-- AGGREGATE FUNCTIONS
-- COUNT(*) / COUNT(column) → BigInt
-- SUM(number) → Number
-- AVG(number) → Double
-- MIN(value) → value type
-- MAX(value) → value type
-- ARRAY_AGG(value) → Array

-- Examples:
SELECT
  PARENT(path) AS parent,
  COUNT(*) AS total_children,
  ARRAY_AGG(name) AS child_names,
  MIN(properties ->> 'created_at') AS earliest,
  MAX(properties ->> 'created_at') AS latest
FROM default
WHERE DEPTH(path) = 2
GROUP BY PARENT(path)
HAVING COUNT(*) > 3;`,
        },
        {
          title: 'Window Functions',
          description: 'Analytical functions with OVER clause',
          sql: `-- WINDOW FUNCTIONS
-- Operate over a window (set of rows) defined by PARTITION BY, ORDER BY, and frame
-- Syntax: function() OVER (PARTITION BY ... ORDER BY ... frame_clause)

-- RANKING FUNCTIONS (no arguments):
-- ROW_NUMBER() → BigInt
--   Sequential number within partition (1, 2, 3, ...)
-- RANK() → BigInt
--   Rank with gaps for ties (1, 2, 2, 4, ...)
-- DENSE_RANK() → BigInt
--   Rank without gaps (1, 2, 2, 3, ...)

-- AGGREGATE WINDOW FUNCTIONS (with arguments):
-- COUNT(*) / COUNT(expr) → BigInt
-- SUM(number) → Number
-- AVG(number) → Double
-- MIN(value) → value type
-- MAX(value) → value type

-- WINDOW CLAUSES:
-- PARTITION BY expr [, ...] - Group rows into partitions
-- ORDER BY expr [ASC|DESC] [, ...] - Define ordering within partition
-- Frame clause (optional):
--   ROWS BETWEEN start AND end
--   RANGE BETWEEN start AND end
--
-- Frame bounds:
--   UNBOUNDED PRECEDING - Start of partition
--   N PRECEDING - N rows/values before current
--   CURRENT ROW - Current row
--   N FOLLOWING - N rows/values after current
--   UNBOUNDED FOLLOWING - End of partition

-- Examples:
SELECT
  id,
  name,
  path,
  PARENT(path) AS parent,
  -- Number rows sequentially within each parent
  ROW_NUMBER() OVER (
    PARTITION BY PARENT(path)
    ORDER BY name
  ) AS row_num,
  -- Count siblings (including self)
  COUNT(*) OVER (PARTITION BY PARENT(path)) AS sibling_count,
  -- Running total ordered by name
  SUM(CAST(properties ->> 'size' AS INT)) OVER (
    PARTITION BY PARENT(path)
    ORDER BY name
    ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
  ) AS running_total
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
LIMIT 10;`,
        },
        {
          title: 'Data Types',
          description: 'Available data types in RaisinDB SQL',
          sql: `-- SCALAR TYPES
-- Int, BigInt, Double, Boolean, Text
-- Uuid, TimestampTz, Path, JsonB

-- COLLECTION TYPES
-- Vector(dims)  -- Fixed-dimension vectors
-- Array(type)   -- Arrays of any type

-- FULL-TEXT TYPES
-- TSVector  -- Tokenized document
-- TSQuery   -- Search query

-- NULLABLE
-- All types can be NULL unless specified otherwise

-- Examples:
SELECT
  id::Text AS id_text,
  CAST(properties ->> 'count' AS Int) AS count_int,
  path::Text AS path_text
FROM default
LIMIT 5;`,
        },
      ],
    },
    {
      id: 'indexing',
      title: 'Indexing & Performance',
      icon: <Zap className="w-4 h-4" />,
      description: 'Configure indexes for fast queries: PROPERTY_INDEX, FULLTEXT, and COMPOUND_INDEX',
      examples: [
        {
          title: 'Property Index (PROPERTY_INDEX)',
          description: 'Create exact-match indexes for fast WHERE clause filtering',
          sql: `-- PROPERTY_INDEX creates a RocksDB exact-match index
-- Ideal for: equality filters, enum values, foreign keys
-- Query pattern: WHERE properties->>'status' = 'published'

CREATE NODETYPE 'shop:Product'
PROPERTIES (
  name String REQUIRED,
  sku String REQUIRED UNIQUE PROPERTY_INDEX,  -- Fast exact match
  category String PROPERTY_INDEX,             -- Fast category filter
  status String DEFAULT 'draft' PROPERTY_INDEX,
  price Number REQUIRED PROPERTY_INDEX,       -- Fast price range
  in_stock Boolean DEFAULT true
);

-- Query using property index (fast):
SELECT id, name, path, properties ->> 'price' AS price
FROM default
WHERE properties ->> 'category' = 'electronics'
  AND properties ->> 'status' = 'published'
ORDER BY properties ->> 'price' DESC;`,
        },
        {
          title: 'Full-Text Index (FULLTEXT)',
          description: 'Enable full-text search with Tantivy indexing',
          sql: `-- FULLTEXT creates a Tantivy full-text index
-- Ideal for: text search, content discovery, fuzzy matching
-- Query pattern: WHERE FULLTEXT_MATCH('search terms', 'english')

CREATE NODETYPE 'cms:Article'
PROPERTIES (
  title String REQUIRED FULLTEXT,    -- Searchable title
  body String FULLTEXT,              -- Searchable content
  tags Array OF String FULLTEXT,     -- Searchable tags
  author String PROPERTY_INDEX,      -- Exact match (not full-text)
  status String DEFAULT 'draft'
);

-- Query using full-text index:
SELECT id, name, path, properties ->> 'title' AS title
FROM default
WHERE FULLTEXT_MATCH('database performance', 'english')
  AND properties ->> 'status' = 'published'
ORDER BY updated_at DESC
LIMIT 20;`,
        },
        {
          title: 'Compound Index (COMPOUND_INDEX)',
          description: 'Create multi-column indexes for efficient ORDER BY + filter queries',
          sql: `-- COMPOUND_INDEX creates a multi-column RocksDB index
-- Ideal for: queries with multiple equality filters + ORDER BY + LIMIT
-- Query pattern: WHERE col1=X AND col2=Y ORDER BY col3 DESC LIMIT N
--
-- Structure: Leading equality columns → optional ordering column
-- System columns: __node_type, __created_at, __updated_at

CREATE NODETYPE 'news:Article'
PROPERTIES (
  title String REQUIRED FULLTEXT,
  category String PROPERTY_INDEX,
  status String DEFAULT 'draft' PROPERTY_INDEX,
  author String PROPERTY_INDEX
)
-- Define compound index for "related articles" queries
COMPOUND_INDEX 'idx_article_category_status_created' ON (
  __node_type,        -- Filter by node type (equality)
  category,           -- Filter by category (equality)
  status,             -- Filter by status (equality)
  __created_at DESC   -- Order by created_at descending
)
PUBLISHABLE
INDEXABLE;

-- This query executes in O(LIMIT) time using the compound index:
SELECT id, path, name, properties ->> 'title' AS title
FROM default
WHERE node_type = 'news:Article'
  AND properties ->> 'category' = 'tech'
  AND properties ->> 'status' = 'published'
ORDER BY created_at DESC
LIMIT 5;`,
        },
        {
          title: 'Multiple Compound Indexes',
          description: 'Define multiple compound indexes for different query patterns',
          sql: `-- Different queries need different index structures
-- Define one compound index per common query pattern

CREATE NODETYPE 'ecommerce:Order'
PROPERTIES (
  customer_id String REQUIRED PROPERTY_INDEX,
  status String DEFAULT 'pending' PROPERTY_INDEX,
  total Number REQUIRED,
  items Array OF Object {}
)
-- Index for customer order history (ORDER BY date DESC)
COMPOUND_INDEX 'idx_customer_orders' ON (
  customer_id,
  __created_at DESC
)
-- Index for admin dashboard (filter by status, ORDER BY date)
COMPOUND_INDEX 'idx_status_orders' ON (
  status,
  __created_at DESC
)
-- Index for reporting (by type and status, ORDER BY total)
COMPOUND_INDEX 'idx_type_status_total' ON (
  __node_type,
  status,
  total DESC
)
INDEXABLE;

-- Each query uses its optimal index:
-- Customer history: idx_customer_orders
SELECT * FROM default
WHERE properties ->> 'customer_id' = 'cust-123'
ORDER BY created_at DESC LIMIT 10;

-- Admin dashboard: idx_status_orders
SELECT * FROM default
WHERE properties ->> 'status' = 'pending'
ORDER BY created_at DESC LIMIT 50;`,
        },
        {
          title: 'Compound Index Column Order',
          description: 'Understanding column ordering for optimal index usage',
          sql: `-- COMPOUND INDEX COLUMN ORDER MATTERS!
--
-- Rule: Equality columns FIRST, ordering column LAST
-- The index can only be used if ALL leading columns have equality predicates
--
-- Index: (category, status, __created_at DESC)
-- ✅ WHERE category='X' AND status='Y' ORDER BY created_at DESC
-- ✅ WHERE category='X' AND status='Y' (no ORDER BY - still uses index)
-- ❌ WHERE status='Y' ORDER BY created_at DESC (missing leading column)
-- ❌ WHERE category='X' ORDER BY created_at DESC (missing middle column)

-- Good: Matches all equality columns
SELECT * FROM default
WHERE properties ->> 'category' = 'tech'
  AND properties ->> 'status' = 'published'
ORDER BY created_at DESC
LIMIT 10;

-- Bad: Missing 'status' equality (can't use index efficiently)
SELECT * FROM default
WHERE properties ->> 'category' = 'tech'
ORDER BY created_at DESC
LIMIT 10;

-- Design indexes based on your most frequent query patterns`,
        },
        {
          title: 'Index Selection Strategy',
          description: 'When to use which index type',
          sql: `-- INDEX SELECTION GUIDE:
--
-- PROPERTY_INDEX:
--   ✓ Single-column equality filters
--   ✓ Enum/status fields
--   ✓ Foreign key lookups
--   ✓ Low cardinality columns
--   Query: WHERE col = 'value'
--
-- FULLTEXT:
--   ✓ Text search with stemming
--   ✓ Boolean query operators (AND, OR, NOT)
--   ✓ Fuzzy matching, wildcards
--   ✓ Content discovery
--   Query: WHERE FULLTEXT_MATCH('terms', 'lang')
--
-- COMPOUND_INDEX:
--   ✓ Multi-column equality + ORDER BY
--   ✓ Pagination queries (LIMIT + offset)
--   ✓ "Top N" queries per group
--   ✓ Time-series queries with filters
--   Query: WHERE a=X AND b=Y ORDER BY c LIMIT N

-- Example: Blog application indexes
CREATE NODETYPE 'blog:Post'
PROPERTIES (
  title String REQUIRED FULLTEXT,     -- Text search
  body String FULLTEXT,               -- Text search
  slug String UNIQUE PROPERTY_INDEX,  -- URL lookup
  author_id String PROPERTY_INDEX,    -- Author filter
  category String PROPERTY_INDEX,     -- Category filter
  status String PROPERTY_INDEX,       -- Status filter
  tags Array OF String FULLTEXT       -- Tag search
)
-- Feed: latest posts by category
COMPOUND_INDEX 'idx_category_feed' ON (
  __node_type, category, status, __created_at DESC
)
-- Author profile: posts by author
COMPOUND_INDEX 'idx_author_posts' ON (
  author_id, status, __created_at DESC
)
PUBLISHABLE;`,
        },
        {
          title: 'System Columns in Indexes',
          description: 'Using __node_type, __created_at, __updated_at in compound indexes',
          sql: `-- SYSTEM COLUMNS FOR COMPOUND INDEXES:
--
-- __node_type:
--   The node type identifier (e.g., 'news:Article')
--   Use as leading column when workspace has mixed types
--
-- __created_at:
--   Node creation timestamp (TIMESTAMPTZ)
--   Use for chronological ordering (feeds, timelines)
--
-- __updated_at:
--   Node last-modified timestamp (TIMESTAMPTZ)
--   Use for "recently updated" queries

-- Timeline feed with type filtering
CREATE NODETYPE 'social:Post'
PROPERTIES (
  content String FULLTEXT,
  author_id String PROPERTY_INDEX
)
COMPOUND_INDEX 'idx_timeline' ON (
  __node_type,
  __created_at DESC
)
COMPOUND_INDEX 'idx_author_timeline' ON (
  author_id,
  __created_at DESC
);

-- Query: Global timeline
SELECT * FROM default
WHERE node_type = 'social:Post'
ORDER BY created_at DESC
LIMIT 20;

-- Query: Author's posts
SELECT * FROM default
WHERE properties ->> 'author_id' = 'user-123'
ORDER BY created_at DESC
LIMIT 20;`,
        },
        {
          title: 'Verify Index Usage with EXPLAIN',
          description: 'Check that your queries are using the expected indexes',
          sql: `-- Use EXPLAIN to verify index usage
-- Look for these scan types in the plan:
--   - PropertyIndexScan: Uses property index
--   - FulltextScan: Uses full-text index
--   - CompoundIndexScan: Uses compound index
--   - PrefixScan: Efficient path prefix scan
--   - TableScan: Full table scan (slowest)

-- Check compound index usage:
EXPLAIN
SELECT id, name, path
FROM default
WHERE node_type = 'news:Article'
  AND properties ->> 'category' = 'tech'
  AND properties ->> 'status' = 'published'
ORDER BY created_at DESC
LIMIT 5;

-- Expected: "CompoundIndexScan" in the plan
-- If you see "TableScan", check that:
-- 1. Index exists with matching columns
-- 2. All leading equality columns are present
-- 3. Column order matches index definition`,
        },
      ],
    },
    {
      id: 'auth',
      title: 'Authentication & Access Control',
      icon: <Shield className="w-4 h-4" />,
      description: 'Query and manage authentication providers, sessions, and permissions',
      examples: [
        {
          title: 'Get Current User',
          description: 'Get the current user\'s node from the access_control workspace (returns JSON)',
          sql: `-- Get current authenticated user's node
-- Returns the full user node as JSON with path, properties, etc.
-- Returns NULL if not authenticated
SELECT RAISIN_CURRENT_USER() AS current_user;

-- Extract just the path
SELECT RAISIN_CURRENT_USER()->>'path' AS user_path;

-- Extract user properties
SELECT RAISIN_CURRENT_USER()->'properties'->>'email' AS email;`,
        },
        {
          title: 'Get Current Workspace Context',
          description: 'Get the active workspace from the request context (planned)',
          sql: `-- Get current workspace context (from X-Raisin-Workspace header)
-- Note: This function is planned but not yet implemented
-- SELECT RAISIN_AUTH_CURRENT_WORKSPACE() AS workspace_id;`,
        },
        {
          title: 'Check Permission',
          description: 'Check if the current user has a specific permission on a resource',
          sql: `-- Check if current user has permission
-- Returns true/false
SELECT RAISIN_AUTH_HAS_PERMISSION('workspace:main', 'read') AS can_read;
SELECT RAISIN_AUTH_HAS_PERMISSION('workspace:main', 'write') AS can_write;
SELECT RAISIN_AUTH_HAS_PERMISSION('workspace:main', 'admin') AS is_admin;`,
        },
        {
          title: 'Get Auth Settings',
          description: 'Retrieve the current tenant authentication settings as JSON',
          sql: `-- Get current authentication configuration
-- Returns JSON with session_duration_hours, password_policy, etc.
SELECT RAISIN_AUTH_GET_SETTINGS() AS auth_settings;`,
        },
        {
          title: 'Update Auth Settings',
          description: 'Update tenant authentication settings (admin only)',
          sql: `-- Update auth settings (requires admin permission)
SELECT RAISIN_AUTH_UPDATE_SETTINGS('{
  "session_duration_hours": 48,
  "max_sessions_per_user": 5
}') AS updated_settings;`,
        },
        {
          title: 'Add OIDC Provider',
          description: 'Add a new authentication provider (Google, Okta, Keycloak, etc.)',
          sql: `-- Add Google OIDC provider
SELECT RAISIN_AUTH_ADD_PROVIDER(
  'oidc:google',
  '{
    "display_name": "Sign in with Google",
    "client_id": "your-client-id.apps.googleusercontent.com",
    "client_secret": "your-client-secret",
    "issuer_url": "https://accounts.google.com",
    "scopes": ["openid", "email", "profile"]
  }'
) AS provider_id;

-- Add Okta OIDC provider
SELECT RAISIN_AUTH_ADD_PROVIDER(
  'oidc:okta',
  '{
    "display_name": "Sign in with Okta",
    "client_id": "0oaXXX",
    "client_secret": "secret",
    "issuer_url": "https://your-org.okta.com",
    "scopes": ["openid", "email", "profile", "groups"]
  }'
) AS provider_id;`,
        },
        {
          title: 'Update Provider Config',
          description: 'Update an existing authentication provider configuration',
          sql: `-- Update provider configuration
SELECT RAISIN_AUTH_UPDATE_PROVIDER(
  'oidc:google',  -- provider_id
  '{
    "enabled": true,
    "display_name": "Continue with Google",
    "scopes": ["openid", "email", "profile", "https://www.googleapis.com/auth/userinfo.email"]
  }'
) AS updated_config;`,
        },
        {
          title: 'Remove Provider',
          description: 'Remove an authentication provider (admin only)',
          sql: `-- Remove an auth provider
-- Returns true if removed, false if not found
SELECT RAISIN_AUTH_REMOVE_PROVIDER('oidc:okta') AS removed;`,
        },
        {
          title: 'Conditional Content by Permission',
          description: 'Filter query results based on user permissions',
          sql: `-- Only show content the user can access
SELECT
  id,
  name,
  path,
  properties
FROM default
WHERE node_type = 'Document'
  AND (
    properties ->> 'visibility' = 'public'
    OR RAISIN_AUTH_HAS_PERMISSION(
      'workspace:' || properties ->> 'workspace',
      'read'
    )
  )
ORDER BY created_at DESC
LIMIT 20;`,
        },
        {
          title: 'Audit User Actions',
          description: 'Include current user context in audit queries',
          sql: `-- Query with user context for auditing
SELECT
  RAISIN_CURRENT_USER()->>'path' AS performed_by,
  id,
  name,
  created_at,
  updated_at
FROM default
WHERE node_type = 'AuditLog'
  AND properties ->> 'action' = 'update'
ORDER BY created_at DESC
LIMIT 50;`,
        },
      ],
    },
    {
      id: 'explain',
      title: 'Query Analysis & Optimization',
      icon: <Zap className="w-4 h-4" />,
      description: 'Analyze query execution plans and understand how queries are executed',
      examples: [
        {
          title: 'Basic EXPLAIN',
          description: 'Show the physical execution plan for a query',
          sql: `-- Show query execution plan
EXPLAIN
SELECT * FROM default
WHERE path LIKE '/content/%'
LIMIT 10;`,
        },
        {
          title: 'EXPLAIN with Verbose Details',
          description: 'Show logical, optimized, and physical plans',
          sql: `-- Show all plan stages (logical, optimized, physical)
EXPLAIN (VERBOSE)
SELECT
  id,
  name,
  path,
  properties ->> 'status' AS status
FROM default
WHERE properties @> '{"status": "published"}'
ORDER BY created_at DESC
LIMIT 20;`,
        },
        {
          title: 'Analyze Join Performance',
          description: 'See how joins are executed and which algorithm is chosen',
          sql: `-- Check join execution strategy
EXPLAIN (VERBOSE)
SELECT
  post.id,
  post.name AS post_title,
  author.properties ->> 'username' AS author_name
FROM default post
LEFT JOIN default author
  ON post.properties ->> 'authorId' = author.id
WHERE post.node_type = 'Post'
LIMIT 10;`,
        },
        {
          title: 'Check Index Usage',
          description: 'Verify that your query is using available indexes',
          sql: `-- Check if property index is being used
EXPLAIN
SELECT * FROM default
WHERE properties ->> 'status' = 'published'
  AND properties ->> 'category' = 'blog';

-- Look for "PropertyIndexScan" in the plan`,
        },
        {
          title: 'Optimize Hierarchy Queries',
          description: 'Verify prefix scan optimization for path queries',
          sql: `-- Verify prefix scan is used (most efficient)
EXPLAIN
SELECT id, name, path
FROM default
WHERE PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY path;

-- Should show "PrefixScan" instead of "TableScan"`,
        },
      ],
    },
  ]

  const filteredSections = sections
    .map(section => ({
      ...section,
      examples: section.examples.filter(example =>
        searchTerm === '' ||
        example.title.toLowerCase().includes(searchTerm.toLowerCase()) ||
        example.description.toLowerCase().includes(searchTerm.toLowerCase()) ||
        example.sql.toLowerCase().includes(searchTerm.toLowerCase())
      ),
    }))
    .filter(section => section.examples.length > 0)

  return (
    <>
      {/* Backdrop */}
      {isOpen && (
        <div
          className="fixed inset-0 bg-black/50 backdrop-blur-sm z-40 animate-fade-in"
          onClick={onClose}
        />
      )}

      {/* Sidebar */}
      <div
        className={`fixed top-0 right-0 h-full w-full md:w-[600px] lg:w-[700px] bg-zinc-900/95 backdrop-blur-xl
                   border-l border-white/10 shadow-2xl z-50 flex flex-col
                   transition-transform duration-300 ease-in-out
                   ${isOpen ? 'translate-x-0' : 'translate-x-full'}`}
      >
        {/* Header */}
        <div className="flex-shrink-0 px-6 py-4 border-b border-white/10 bg-gradient-to-r from-black/40 to-black/30">
          <div className="flex items-center justify-between mb-4">
            <div className="flex items-center gap-3">
              <div className="p-2 bg-primary-500/10 rounded-lg border border-primary-500/20">
                <Zap className="w-5 h-5 text-primary-400" />
              </div>
              <div>
                <h2 className="text-xl font-bold text-white">SQL Help & Examples</h2>
                <p className="text-sm text-zinc-400 mt-0.5">
                  Interactive guide for {repo}
                </p>
              </div>
            </div>
            <button
              onClick={onClose}
              className="text-zinc-400 hover:text-white transition-colors p-2 hover:bg-white/10 rounded-lg"
            >
              <X className="w-5 h-5" />
            </button>
          </div>

          {/* Search */}
          <div className="relative">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-zinc-500" />
            <input
              type="text"
              placeholder="Search examples..."
              value={searchTerm}
              onChange={(e) => setSearchTerm(e.target.value)}
              className="w-full pl-10 pr-4 py-2.5 bg-black/40 border border-white/10 rounded-lg
                       text-white placeholder-zinc-500 focus:outline-none focus:ring-2
                       focus:ring-primary-500/50 focus:border-primary-500/50 transition-all"
            />
          </div>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-6 py-4">
          {filteredSections.length === 0 ? (
            <div className="text-center py-12">
              <Search className="w-12 h-12 mx-auto mb-3 text-zinc-600" />
              <p className="text-zinc-400">No examples found matching "{searchTerm}"</p>
            </div>
          ) : (
            <div className="space-y-4">
              {filteredSections.map((section) => (
                <div
                  key={section.id}
                  className="bg-white/5 backdrop-blur-sm border border-white/10 rounded-xl overflow-hidden"
                >
                  {/* Section Header */}
                  <button
                    onClick={() => toggleSection(section.id)}
                    className="w-full px-5 py-4 flex items-center justify-between hover:bg-white/5 transition-colors"
                  >
                    <div className="flex items-center gap-3">
                      <div className="p-2 bg-primary-500/10 rounded-lg border border-primary-500/20">
                        {section.icon}
                      </div>
                      <div className="text-left">
                        <h3 className="text-sm font-semibold text-white">{section.title}</h3>
                        <p className="text-xs text-zinc-400 mt-0.5">{section.description}</p>
                      </div>
                    </div>
                    {expandedSections[section.id] ? (
                      <ChevronDown className="w-5 h-5 text-zinc-400" />
                    ) : (
                      <ChevronRight className="w-5 h-5 text-zinc-400" />
                    )}
                  </button>

                  {/* Examples */}
                  {expandedSections[section.id] && (
                    <div className="border-t border-white/10 bg-black/20 p-4 space-y-3">
                      {section.examples.map((example, idx) => (
                        <div
                          key={idx}
                          className="bg-black/40 border border-white/10 rounded-lg overflow-hidden
                                   hover:border-primary-500/30 transition-all group"
                        >
                          <div className="px-4 py-3 border-b border-white/10 bg-gradient-to-r from-black/30 to-transparent">
                            <div className="flex items-start justify-between gap-3">
                              <div className="flex-1">
                                <h4 className="text-sm font-semibold text-white group-hover:text-primary-300 transition-colors">
                                  {example.title}
                                </h4>
                                <p className="text-xs text-zinc-400 mt-1">{example.description}</p>
                              </div>
                              <button
                                onClick={() => handleInsert(example.sql)}
                                className="flex items-center gap-1.5 px-3 py-1.5 bg-primary-500/20
                                         hover:bg-primary-500/30 border border-primary-500/30
                                         hover:border-primary-500/50 text-primary-300 hover:text-primary-200
                                         rounded-lg text-xs font-medium transition-all
                                         focus:outline-none focus:ring-2 focus:ring-primary-500/50
                                         active:scale-95"
                              >
                                <Code2 className="w-3.5 h-3.5" />
                                Insert
                              </button>
                            </div>
                          </div>
                          <pre className="px-4 py-3 overflow-x-auto text-xs font-mono text-zinc-300 bg-black/30">
                            <code>{example.sql}</code>
                          </pre>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex-shrink-0 px-6 py-3 border-t border-white/10 bg-gradient-to-r from-black/40 to-black/30">
          <div className="flex items-center gap-2 text-xs text-zinc-500">
            <Database className="w-4 h-4" />
            <span>Hierarchical, versioned database • Click "Insert" to use examples</span>
          </div>
        </div>
      </div>
    </>
  )
}
