export function sqlKnowledge(): string {
  return `# RaisinDB SQL Reference

## Basics

Workspaces are used as table names. Quote names containing colons.

\`\`\`sql
SELECT id, path, name, node_type, properties FROM 'my-workspace'
SELECT * FROM "raisin:access_control" WHERE node_type = 'raisin:User'
\`\`\`

## CRUD Operations

\`\`\`sql
-- Insert (path is required, name is derived from last path segment)
INSERT INTO 'workspace' (path, node_type, properties)
VALUES ('/folder/item-1', 'myapp:Article', '{"title":"Hello"}'::jsonb)

-- Update properties (merge with ||)
UPDATE 'workspace' SET properties = properties || '{"title":"Updated"}'::jsonb WHERE path = '/folder/item-1'

-- Replace properties entirely
UPDATE 'workspace' SET properties = '{"title":"Replaced"}'::jsonb WHERE path = '/folder/item-1'

-- Delete
DELETE FROM 'workspace' WHERE path = '/folder/item-1'
\`\`\`

## Parameterized Queries

Use $1, $2, etc. for bind parameters:

\`\`\`sql
SELECT * FROM "raisin:access_control" WHERE properties ->> 'email' = $1
INSERT INTO 'workspace' (path, node_type, properties) VALUES ($1, $2, $3::jsonb)
\`\`\`

## JSON Property Access

The \`->>\` operator extracts text from JSONB properties. Works directly without casts.

\`\`\`sql
-- Text extraction
SELECT * FROM 'workspace' WHERE properties ->> 'status' = 'active'

-- Nested access
SELECT properties ->> 'title' AS title FROM 'workspace'

-- Containment (@>) and key existence (?)
SELECT * FROM 'workspace' WHERE properties @> '{"role":"admin"}'::jsonb
SELECT * FROM 'workspace' WHERE properties ? 'email'

-- JSON_VALUE and JSON_EXISTS
SELECT JSON_VALUE(properties, '$.metadata.author') FROM 'workspace'
SELECT * FROM 'workspace' WHERE JSON_EXISTS(properties, '$.tags')

-- Typed extraction
SELECT JSON_GET_INT(properties, '$.rating') FROM 'workspace'
SELECT JSON_GET_BOOL(properties, '$.featured') FROM 'workspace'
\`\`\`

## Hierarchy Functions

RaisinDB paths form a tree. Use these functions to query hierarchy:

\`\`\`sql
-- Direct children of a path
SELECT * FROM 'workspace' WHERE CHILD_OF('/content')

-- All descendants (any depth)
SELECT * FROM 'workspace' WHERE DESCENDANT_OF('/content')

-- Descendants with max depth
SELECT * FROM 'workspace' WHERE DESCENDANT_OF('/content', 2)

-- Path prefix matching
SELECT * FROM 'workspace' WHERE PATH_STARTS_WITH('/blog/posts')

-- Get depth level of a node
SELECT path, DEPTH(path) AS level FROM 'workspace'

-- Navigate up the tree
SELECT PARENT(path) AS parent_path FROM 'workspace' WHERE path = '/a/b/c'
SELECT ANCESTOR(path, 2) AS grandparent FROM 'workspace' WHERE path = '/a/b/c'
\`\`\`

## References (raisin:ref)

A Reference property stores a link to another node as a JSON object:

\`\`\`json
{
  "raisin:ref": "node-id-or-path",
  "raisin:workspace": "workspace-name",
  "raisin:path": "/path/to/node"
}
\`\`\`

Fields:
- \`raisin:ref\` (required) -- node ID (UUID/nanoid) or path (starts with \`/\`)
- \`raisin:workspace\` (required) -- target workspace name
- \`raisin:path\` (optional) -- auto-populated during write if ref contains a path

Store a reference in properties:

\`\`\`sql
INSERT INTO 'workspace' (path, node_type, properties)
VALUES ('/posts/my-post', 'myapp:Post', '{
  "title": "My Post",
  "author": {
    "raisin:ref": "/users/john",
    "raisin:workspace": "raisin:access_control"
  }
}'::jsonb)
\`\`\`

### RESOLVE -- Dereference References

Resolves all \`raisin:ref\` objects in a JSONB value, replacing them with the referenced node's properties:

\`\`\`sql
-- Resolve all references at depth 1 (default)
SELECT RESOLVE(properties) FROM 'workspace' WHERE id = 'node-123'

-- Resolve nested references up to depth 3 (max 10)
SELECT RESOLVE(properties, 3) FROM 'workspace' WHERE path = '/posts/my-post'
\`\`\`

### REFERENCES -- Find Nodes Referencing a Target

\`\`\`sql
SELECT * FROM 'workspace' WHERE REFERENCES('media:/images/header-image')
\`\`\`

## Graph Relations (RELATE / UNRELATE)

Create typed, weighted edges between any two nodes (even across workspaces):

### RELATE

\`\`\`sql
-- Create a relationship
RELATE FROM path='/articles/post-1' TO path='/tags/tech' TYPE 'tagged';

-- With weight
RELATE FROM path='/articles/post-1' TO path='/tags/tech' TYPE 'tagged' WEIGHT 2.0;

-- Cross-workspace
RELATE
  FROM path='/content/page' IN WORKSPACE 'main'
  TO path='/assets/hero.jpg' IN WORKSPACE 'media'
  TYPE 'uses_asset';

-- Using node IDs
RELATE FROM id='abc-123' TO id='def-456' TYPE 'follows';

-- With branch override
RELATE IN BRANCH 'feature-x'
  FROM path='/articles/post-1' TO path='/tags/tech' TYPE 'tagged';
\`\`\`

### UNRELATE

\`\`\`sql
-- Remove a specific relationship
UNRELATE FROM path='/articles/post-1' TO path='/tags/tech' TYPE 'tagged';

-- Remove any relationship between two nodes
UNRELATE FROM path='/articles/post-1' TO path='/tags/tech';
\`\`\`

### Query Relations with NEIGHBORS

\`\`\`sql
SELECT NEIGHBORS(id, 'OUT', 'tagged') FROM 'workspace' WHERE path = '/articles/post-1'
SELECT NEIGHBORS(id, 'IN', 'follows') FROM 'workspace' WHERE path = '/users/alice'
SELECT NEIGHBORS(id, 'BOTH', 'friends') FROM 'workspace' WHERE path = '/users/bob'
\`\`\`

Parameters: \`NEIGHBORS(node_id, direction, relation_type)\`
- direction: \`'OUT'\` (outgoing), \`'IN'\` (incoming), \`'BOTH'\`

### RELATES Expression (in WHERE)

\`\`\`sql
-- Check if nodes are related
WHERE node.parent RELATES target_id VIA 'FRIENDS_WITH' DEPTH 1..2
WHERE user_id RELATES target_user DEPTH 1..3 DIRECTION OUTGOING
\`\`\`

Direction: \`OUTGOING\`, \`INCOMING\`, \`ANY\` (default)

### Graph Queries with GRAPH_TABLE

\`\`\`sql
SELECT * FROM GRAPH_TABLE(
  MATCH (a:User)-[:follows]->(b:User)
  WHERE a.id = 'alice'
  COLUMNS (a.name, b.name AS friend_name)
);
\`\`\`

Pattern syntax:
- \`(var:Type)\` -- node with optional type label
- \`-[:RelationType]->\` -- directed outgoing
- \`<-[:RelationType]-\` -- directed incoming

## MOVE / COPY / ORDER

### MOVE

Relocate a node and all descendants to a new parent:

\`\`\`sql
MOVE workspace SET path='/articles/old-post' TO path='/archive/2024'
MOVE workspace SET id='abc123' TO path='/target/parent'
MOVE workspace IN BRANCH 'feature-x' SET path='/source' TO path='/target'
\`\`\`

### COPY / COPY TREE

Duplicate a node (generates new IDs):

\`\`\`sql
-- Copy single node
COPY workspace SET path='/templates/page' TO path='/content' AS 'new-page'

-- Copy entire subtree recursively
COPY TREE workspace SET path='/templates/section' TO path='/content'
\`\`\`

### ORDER

Reorder siblings within a parent:

\`\`\`sql
ORDER workspace SET path='/items/b' ABOVE path='/items/a'
ORDER workspace SET path='/items/c' BELOW path='/items/a'
\`\`\`

## RESTORE

Restore a node (or tree) to a previous revision:

\`\`\`sql
-- Restore single node
RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2

-- Restore entire tree
RESTORE TREE NODE path='/products/category' TO REVISION HEAD~5

-- Restore with specific translation locales
RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2 TRANSLATIONS ('en', 'de')
\`\`\`

Revision references: \`HEAD~N\` (N revisions back) or HLC timestamp.

## Translations (UPDATE FOR LOCALE)

Update translated properties:

\`\`\`sql
-- Simple property
UPDATE 'workspace' FOR LOCALE 'de' SET title = 'Titel' WHERE path = '/post'

-- Nested property
UPDATE 'workspace' FOR LOCALE 'fr' SET metadata.author = 'Jean' WHERE id = 'abc'

-- Block property in a SectionField element
UPDATE 'workspace' FOR LOCALE 'de'
  SET blocks[uuid='550e8400'].text = 'Hallo'
  WHERE path = '/post'
\`\`\`

## Full-Text Search

\`\`\`sql
SELECT * FROM 'workspace' WHERE FULLTEXT_MATCH('database management', 'english')
\`\`\`

Requires \`index: [Fulltext]\` on the property in the NodeType definition.

## Vector / Embedding Functions

\`\`\`sql
-- Generate embedding from text (async, uses configured AI provider)
SELECT EMBEDDING('machine learning concepts') AS vec

-- Vector distance calculations
SELECT *, VECTOR_L2_DISTANCE(embedding_col, EMBEDDING('search query')) AS dist
FROM 'workspace'
ORDER BY dist ASC
LIMIT 10

-- Other distance functions
VECTOR_COSINE_DISTANCE(vector1, vector2)
VECTOR_INNER_PRODUCT(vector1, vector2)
\`\`\`

## Filtering and Sorting

\`\`\`sql
SELECT * FROM 'workspace'
WHERE node_type = 'myapp:Article'
  AND properties ->> 'status' = 'published'
ORDER BY properties ->> 'created_at' DESC
LIMIT 10

-- LIKE on text properties
SELECT * FROM 'workspace' WHERE properties ->> 'title' LIKE '%search%'
\`\`\`

## Mixin DDL

\`\`\`sql
-- Create a mixin (reusable property set)
CREATE MIXIN 'myapp:SEOFields' DESCRIPTION 'Common SEO properties' PROPERTIES (
  meta_title String,
  meta_description String,
  og_image Resource
);

-- Alter a mixin
ALTER MIXIN 'myapp:SEOFields' ADD PROPERTY canonical_url String;

-- Drop a mixin (CASCADE removes it from all referencing NodeTypes)
DROP MIXIN 'myapp:SEOFields' CASCADE;
\`\`\`

## Built-in Columns

Every node exposes these columns:
- \`id\` -- unique node ID
- \`path\` -- full hierarchical path
- \`name\` -- node name (last path segment)
- \`node_type\` -- the NodeType name
- \`archetype\` -- archetype name (if set)
- \`properties\` -- JSONB properties object
- \`revision\` -- version number
- \`created_at\`, \`updated_at\` -- timestamps
`;
}
