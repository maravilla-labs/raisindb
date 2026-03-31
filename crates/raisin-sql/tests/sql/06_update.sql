-- UPDATE statements for modifying nodes

-- Basic UPDATE with properties merge
UPDATE nodes
SET properties = properties || '{"status": "published"}'
WHERE id = 'node-xyz-123';

-- UPDATE with optimistic concurrency control
UPDATE nodes
SET
    properties = properties || '{"status": "published", "tags": ["rust", "database"]}',
    updated_by = 'editor-1'
WHERE id = 'node-xyz-123' AND version = 5;

-- UPDATE single property
UPDATE nodes
SET properties = properties || '{"status": "archived"}'
WHERE id = 'node-abc-456';

-- UPDATE with complex JSON merge
UPDATE nodes
SET properties = properties || '{"metadata": {"featured": true, "priority": "high"}}'
WHERE path = '/content/blog/important-post';

-- UPDATE based on path pattern
UPDATE nodes
SET properties = properties || '{"category": "legacy"}'
WHERE PATH_STARTS_WITH(path, '/archive/');

-- UPDATE multiple fields
UPDATE nodes
SET
    properties = properties || '{"reviewed": true}',
    updated_by = 'reviewer-1',
    version = version + 1
WHERE id = 'node-def-789';

-- UPDATE with current timestamp
UPDATE nodes
SET
    properties = properties || '{"last_reviewed": "2025-10-21"}',
    updated_by = 'admin'
WHERE properties ->> 'status' = 'pending_review';

-- UPDATE nested JSON property
UPDATE nodes
SET properties = properties || '{"seo": {"title": "Updated SEO Title", "keywords": ["new", "updated"]}}'
WHERE id = 'node-123';

-- UPDATE with version check for concurrency
UPDATE nodes
SET
    properties = properties || '{"price": 999.99}',
    updated_by = 'pricing-bot'
WHERE id = 'product-123'
AND version = 10;

-- UPDATE array property
UPDATE nodes
SET properties = properties || '{"tags": ["updated", "modified", "new"]}'
WHERE path = '/content/blog/post-1';

-- Bulk UPDATE based on node_type
UPDATE nodes
SET properties = properties || '{"migrated": true}'
WHERE node_type = 'my:LegacyArticle';

-- UPDATE with JSON removal (setting to null)
UPDATE nodes
SET properties = properties || '{"deprecated_field": null}'
WHERE id = 'node-456';

-- UPDATE translation
UPDATE nodes
SET translations = translations || '{"es": {"title": "Hola Mundo"}}'
WHERE id = 'node-789';

-- UPDATE published state
UPDATE nodes
SET
    properties = properties || '{"status": "published"}',
    published_by = 'publisher-1'
WHERE id = 'article-123'
AND properties ->> 'status' = 'approved';

-- UPDATE with hierarchy check
UPDATE nodes
SET properties = properties || '{"level": 3}'
WHERE DEPTH(path) = 3 AND PATH_STARTS_WITH(path, '/content/');
