# events -- RaisinDB Package

When generating YAML or configuration, prefer structured output that can be validated.

## Validation

Before committing or building, always validate:

```bash
raisindb package create --check .
```

This checks manifest references, YAML syntax, and workspace consistency. Fix all errors before proceeding.

## Content-Driven Application Design

RaisinDB uses a three-layer content model:

- **NodeType** (schema) -- defines what properties content CAN have, like a database table
- **Archetype** (page template) -- defines a specific page type: which fields appear in the
  admin editor AND which frontend component renders it. Links to a base NodeType.
- **ElementType** (content block) -- a composable block placed inside SectionFields of an
  archetype. Each maps to a frontend component (Hero, TextBlock, FeatureGrid, etc.)

The frontend maps archetypes and element types to components:
```
NodeType (schema) → Archetype (page template) → ElementTypes (blocks)
     ↓                      ↓                         ↓
  Database           Page Component            Element Components
```

## Package Structure

```
package/
├── manifest.yaml           # Package metadata and provides declarations
├── workspaces/
│   └── events.yaml  # Workspace definition (allowed types, root structure)
├── nodetypes/              # NodeType YAML -- data schemas
├── archetypes/             # Archetype YAML -- page templates (editor fields + frontend component)
├── elementtypes/           # ElementType YAML -- composable content blocks (frontend components)
├── content/
│   ├── events/      # Initial workspace content
│   └── functions/          # Server-side functions and triggers
│       ├── lib/            # Function implementations
│       └── triggers/       # Event triggers
└── static/                 # Static assets (images, files)
```

## Reference

Detailed guides are available in `.agent/knowledge/`:

- `node-types.md` -- NodeTypes, Archetypes, ElementTypes, property types, Reference format
- `sql.md` -- SQL syntax, RELATE/UNRELATE, MOVE/COPY, RESOLVE, graph queries, hierarchy, vectors
- `triggers.md` -- trigger configuration, event filters
- `flows.md` -- multi-step workflow definitions
- `functions/` -- server-side function patterns (JS and Starlark)
- `sdk/` -- client SDK: connection, auth, node CRUD, events, SQL, flows

## Frontend SDK Setup

Connect to RaisinDB from your frontend using the `@raisindb/client` SDK:

```typescript
import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';

const client = new RaisinClient('ws://localhost:8081/sys/default/events', {
  tokenStorage: new LocalStorageTokenStorage('events'),
  tenantId: 'default',
  defaultBranch: 'main',
});

// Initialize session on app start (restores auth from localStorage)
const user = await client.initSession('events');

// Auth: loginWithEmail, registerWithEmail, logout, onAuthStateChange
// See .agent/knowledge/sdk/overview.md for full auth patterns

// Query content
const db = client.database('events');
const result = await db.executeSql(
  "SELECT * FROM 'events' WHERE node_type = $1", ['events:Article']
);
```

## Frontend Component Mapping

Map archetypes to page components and element types to block components:

```typescript
// components/pages/index.ts -- archetype → page component
export const pageComponents: Record<string, ComponentType> = {
  'events:BlogPost': BlogPostPage,
  'events:LandingPage': LandingPage,
};

// components/elements/index.ts -- element type → block component
export const elementComponents: Record<string, ComponentType> = {
  'events:Hero': Hero,
  'events:TextBlock': TextBlock,
};

// Page component renders its elements (any framework):
function renderPage(page) {
  return (page.properties.content ?? []).map(element => {
    const Component = elementComponents[element.element_type];
    return Component ? <Component element={element} /> : null;
  });
}
```

## Quick-Start Patterns

### Create a NodeType + Archetype

```yaml
# nodetypes/article.yaml -- data schema
name: events:Article
title: Article
properties:
  - name: title
    type: String
    required: true
    index: [Fulltext]
  - name: slug
    type: String
    required: true
  - name: body
    type: String
  - name: author
    type: Reference        # Links to another node via raisin:ref
versionable: true
```

```yaml
# archetypes/blog-post.yaml -- page template
name: events:BlogPost
title: Blog Post
base_node_type: events:Article
fields:
  - $type: TextField
    name: title
    required: true
  - $type: TextField
    name: slug
  - $type: SectionField
    name: content
    title: Content
    allowed_element_types:
      - events:TextBlock
```

### Write SQL Queries

```sql
-- Query content
SELECT id, path, archetype, properties
FROM 'events'
WHERE node_type = 'events:Article'
  AND properties ->> 'title' LIKE '%search%'
ORDER BY properties ->> 'created_at' DESC

-- Create graph relations between nodes
RELATE FROM path='/articles/post-1' TO path='/tags/tech' TYPE 'tagged';

-- Move nodes
MOVE events SET path='/articles/old' TO path='/archive/2024'

-- Resolve references (replace raisin:ref objects with actual data)
SELECT RESOLVE(properties) FROM 'events' WHERE path = '/posts/my-post'
```

### Create a Trigger

```yaml
# content/functions/triggers/on-article-created/.node.yaml
node_type: raisin:Trigger
properties:
  title: On Article Created
  enabled: true
  trigger_type: node_event
  config:
    event_kinds: [Created]
  filters:
    workspaces: ["events"]
    node_types: ["events:Article"]
  function_path: /lib/events/handle-article-created
```
