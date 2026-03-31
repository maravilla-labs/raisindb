export function createNodeTypeTask(): string {
  return `# Task: Create a Node Type

## Objective

Create a new NodeType schema that defines the data structure for a content type.

## Steps

### 1. Design the Schema

Decide on:
- **Name**: Use namespace prefix, e.g. \`myapp:Article\`
- **Purpose**: What content does this type represent?
- **Properties**: What fields does it need? Choose appropriate types:
  - \`String\` -- text values
  - \`Int\` / \`Float\` -- numeric values
  - \`Boolean\` -- true/false flags
  - \`DateTime\` -- timestamps
  - \`Reference\` -- link to another node
  - \`Section\` -- container for ElementType blocks
  - \`StringList\` / \`ReferenceList\` -- arrays
  - \`Json\` -- freeform structured data
- **Indexes**: Which properties should be indexed for queries?

### 2. Create the NodeType File

Create \`nodetypes/{namespace}:{Name}.yaml\`:

\`\`\`yaml
name: "{namespace}:{Name}"
title: "{Human Readable Title}"
description: "What this type represents"
icon: "file-text"

properties:
  - name: title
    type: String
    title: Title
    required: true
    indexed: true

  - name: slug
    type: String
    title: URL Slug
    indexed: true

  - name: description
    type: String
    title: Description

  - name: content
    type: Section
    title: Content
\`\`\`

### 3. Create an Archetype (If This Is a Page Type)

If this NodeType will be rendered as a page, create an archetype in
\`archetypes/{namespace}:{Name}.yaml\`. The archetype links to the NodeType via
\`base_node_type\`, defines editor fields (\`$type: TextField\`, \`SectionField\`, etc.),
and specifies which ElementTypes can be placed in sections. In the frontend, map the
archetype to a page component: \`pageComponents['ns:Name'] = MyPageComponent\`.

Also register the archetype in \`manifest.yaml\` under \`provides.archetypes\`.

### 4. Register in manifest.yaml

Add the NodeType name to the \`provides.nodetypes\` list:

\`\`\`yaml
provides:
  nodetypes:
    - {namespace}:{Name}
\`\`\`

### 5. Add to Workspace (If Needed)

If content of this type should be creatable in a workspace, add it to the
workspace's \`allowed_node_types\` list in \`workspaces/{workspace}.yaml\`.

### 6. Validate

Run validation to catch errors:

\`\`\`bash
raisindb package create --check .
\`\`\`

## Reference

See \`.agent/knowledge/node-types.md\` for detailed NodeType format documentation.

---

# Task: Create a Mixin

## Objective

Create a reusable property set (Mixin) that can be composed into multiple NodeTypes.

## Steps

### 1. Identify Shared Properties

Look for properties that are duplicated across multiple NodeTypes. Common examples:
- SEO fields (meta_title, meta_description, og_image)
- Audit fields (created_by, updated_by, approved_by)
- Social metadata (share_count, like_count)
- Geolocation (latitude, longitude, address)

### 2. Create the Mixin File

Create \`mixins/{namespace}:{Name}.yaml\`:

\`\`\`yaml
name: "{namespace}:{Name}"
title: "{Human Readable Title}"
description: "What shared properties this mixin provides"
is_mixin: true

properties:
  - name: meta_title
    type: String
    title: Meta Title
  - name: meta_description
    type: String
    title: Meta Description
\`\`\`

### 3. Register in manifest.yaml

Add the Mixin name to the \`provides.mixins\` list:

\`\`\`yaml
provides:
  mixins:
    - {namespace}:{Name}
\`\`\`

### 4. Validate

\`\`\`bash
raisindb package create --check .
\`\`\`

## SQL Alternative

You can also create mixins via SQL:

\`\`\`sql
CREATE MIXIN 'myapp:SEOFields' DESCRIPTION 'Common SEO properties' PROPERTIES (
  meta_title String,
  meta_description String
);
\`\`\`
`;
}
