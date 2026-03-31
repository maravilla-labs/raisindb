# RaisinDB Node Types Reference

## NodeType Definition

NodeTypes define the schema for content stored in RaisinDB. Defined in YAML:

```yaml
name: myapp:Article
title: Article
description: A blog article
icon: file-text
color: "#6366f1"
version: 1

properties:
  - name: title
    title: Title
    type: String
    required: true
    is_translatable: true
    index:
      - Fulltext
  - name: slug
    title: Slug
    type: String
    required: true
  - name: body
    title: Body
    type: String
    required: false
    is_translatable: true
  - name: tags
    title: Tags
    type: Array
    required: false
  - name: metadata
    title: Metadata
    type: Object
    required: false
  - name: published_at
    title: Published Date
    type: Date
  - name: view_count
    title: Views
    type: Number
    default: 0
  - name: featured
    title: Featured
    type: Boolean
    default: false
  - name: author
    title: Author
    type: Reference
    required: false
  - name: cover_image
    title: Cover Image
    type: Resource
    required: false

versionable: true
publishable: true
auditable: true
indexable: true
```

## Property Types

| Type | Description | Stored as |
|------|-------------|-----------|
| String | Text value | JSON string |
| Number | Numeric value (integer or float) | JSON number |
| Boolean | true/false | JSON boolean |
| Date | ISO 8601 date/datetime | JSON string |
| Object | Nested JSON object | JSON object |
| Array | JSON array | JSON array |
| Reference | Link to another node | raisin:ref object (see below) |
| Resource | Binary resource / file attachment | Resource metadata object |

## Reference Type (raisin:ref)

A Reference property stores a link to another node. The value is a JSON object
with special keys:

```json
{
  "raisin:ref": "node-id-or-path",
  "raisin:workspace": "workspace-name",
  "raisin:path": "/optional/path/to/node"
}
```

- `raisin:ref` (required) -- the node ID (UUID/nanoid) or a path (starting with `/`)
- `raisin:workspace` (required) -- the workspace where the referenced node lives
- `raisin:path` (optional) -- auto-populated on write when ref is a path

Example in a NodeType:
```yaml
properties:
  - name: author
    type: Reference     # Stores a raisin:ref object
    required: false
  - name: related_posts
    type: Array          # Array of raisin:ref objects
    required: false
```

Setting a reference via SQL:
```sql
UPDATE 'workspace' SET properties = properties || '{
  "author": {
    "raisin:ref": "/users/john",
    "raisin:workspace": "raisin:access_control"
  }
}'::jsonb WHERE path = '/posts/my-post'
```

Resolving references (replace ref objects with actual node data):
```sql
SELECT RESOLVE(properties) FROM 'workspace' WHERE path = '/posts/my-post'
SELECT RESOLVE(properties, 3) FROM 'workspace' WHERE path = '/posts/my-post'  -- depth 3
```

## Property Fields

| Field | Type | Description |
|-------|------|-------------|
| name | string | Property key (used in queries and code) |
| title | string | Display label in UI |
| type | string | One of the property types above |
| required | boolean | Whether the property must be set |
| default | any | Default value when not provided |
| is_translatable | boolean | Enable i18n translation |
| index | string[] | Index types, e.g. `[Fulltext]` |

## NodeType Flags

| Flag | Effect |
|------|--------|
| versionable | Tracks revision history, enables draft/publish workflow |
| publishable | Adds publish/unpublish lifecycle |
| auditable | Records who changed what and when |
| indexable | Includes in full-text search index |

## Archetype (Page Template)

Archetypes define a page type. They link a base NodeType to editor fields and to a
frontend page component. Multiple archetypes can share one NodeType for different layouts.

```yaml
name: myapp:BlogPost
title: Blog Post
base_node_type: myapp:Article    # Links to the NodeType

fields:
  - $type: TextField
    name: title
    title: Title
    required: true
    translatable: true
  - $type: TextField
    name: slug
    title: URL Slug
    required: true
  - $type: SectionField             # Container for element blocks
    name: content
    title: Page Content
    allowed_element_types:           # What blocks can be added here
      - myapp:TextBlock
      - myapp:ImageBlock

publishable: true
```

Field types: TextField, TextareaField, NumberField, BooleanField, DateField,
MediaField, SectionField, CompositeField.

In the frontend, map the archetype to a page component:
```typescript
// components/pages/index.ts -- works with any framework
export const pageComponents: Record<string, ComponentType> = {
  'myapp:BlogPost': BlogPostPage,
  'myapp:LandingPage': LandingPage,
};
```

## ElementType (Composable Content Block)

ElementTypes are composable blocks placed inside SectionFields. Each maps to a
frontend component. A page stores its content as an array of elements, each with
a `uuid` and `element_type`.

```yaml
name: myapp:TextBlock
title: Text Block
icon: type

fields:
  - $type: TextField
    name: heading
    title: Heading
    translatable: true
  - $type: TextareaField
    name: body
    title: Content
    required: true
    translatable: true
```

In the frontend, map element types to components:
```typescript
// components/elements/index.ts -- works with any framework
export const elementComponents: Record<string, ComponentType> = {
  'myapp:TextBlock': TextBlock,
  'myapp:Hero': Hero,
  'myapp:FeatureGrid': FeatureGrid,
};
```

A page component renders its elements by iterating over the content array:
```typescript
// Pseudocode -- adapt to your framework (React, Svelte, Vue, etc.)
function renderPage(page) {
  const elements = page.properties.content ?? [];
  return elements.map(element => {
    const Component = elementComponents[element.element_type];
    return Component ? <Component element={element} /> : null;
  });
}
```

## Data Flow

```
NodeType (schema) → Archetype (page template) → ElementTypes (blocks)
     ↓                      ↓                         ↓
  Database           Page Component            Element Components
```

## File Locations

- NodeTypes: `package/nodetypes/<name>.yaml`
- Archetypes: `package/archetypes/<name>.yaml`
- ElementTypes: `package/elementtypes/<name>.yaml`
- Register all in `package/manifest.yaml` under `provides:`
