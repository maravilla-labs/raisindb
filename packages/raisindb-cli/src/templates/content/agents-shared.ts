export function principlesMd(): string {
  return `# Content-Driven Design Principles

## Everything Is a Content Node

All data in RaisinDB is stored as content nodes. A node has a type (NodeType),
properties (typed key-value pairs), and a position in a tree hierarchy. There are
no separate "database tables" -- the node tree IS the database.

## Schema-First via NodeTypes

Every node must conform to a NodeType schema. Define your data model in
\`nodetypes/*.yaml\` before creating content. The schema declares property names,
types, validation rules, and indexing. This ensures data integrity at the storage
layer.

## Reusable Property Sets via Mixins

Mixins are reusable property sets that can be composed into multiple NodeTypes.
Define common property groups (e.g. SEO fields, audit fields, social metadata)
once in \`mixins/*.yaml\` and reference them from any NodeType. This avoids
duplicating property definitions across types. Mixins are defined using
\`CREATE MIXIN\` in SQL or as YAML files in the \`mixins/\` directory.

## Workspace Isolation

Content lives inside workspaces. Each workspace declares which NodeTypes are
allowed and what the root folder structure looks like. Different workspaces can
serve different purposes (public site, admin data, user content) while sharing
the same NodeType definitions.

## Event-Driven with Triggers

Instead of polling for changes, subscribe to content events using triggers.
When a node is created, updated, or deleted, triggers fire and invoke handler
functions. This keeps logic decoupled and reactive.

## Flows Orchestrate Multi-Step Processes

For anything beyond a single function call, use flows. Flows coordinate
sequences of steps -- function calls, human tasks, AI interactions, waits, and
conditional branching. They persist state across steps and support compensation
(rollback) when things go wrong.

## Graph Relationships Connect Nodes

Nodes can reference each other using Reference properties. This creates a graph
of relationships on top of the tree hierarchy. Use References for cross-cutting
concerns like tags, categories, authors, and related content.
`;
}

export function repoMapMd(): string {
  return `# Package Directory Structure

\`\`\`
package/
├── manifest.yaml              # Package metadata, version, provides list
├── README.md                  # Package documentation
│
├── nodetypes/                 # Data schemas
│   └── {namespace}:{Name}.yaml  # One file per NodeType
│
├── mixins/                    # Reusable property sets
│   └── {namespace}:{Name}.yaml  # One file per Mixin
│
├── archetypes/                # Page templates (editor fields + frontend component mapping)
│   └── {namespace}:{Name}.yaml  # One file per Archetype
│
├── elementtypes/              # Composable content blocks (mapped to frontend components)
│   └── {namespace}:{Name}.yaml  # One file per ElementType
│
├── workspaces/                # Workspace definitions
│   └── {name}.yaml            # Allowed types, root structure
│
├── content/
│   ├── {workspace}/           # Initial content nodes (tree of .node.yaml)
│   │   └── folder/
│   │       └── .node.yaml
│   └── functions/
│       ├── lib/               # Library functions (reusable logic)
│       │   └── {namespace}/
│       │       └── {fn-name}/
│       │           ├── .node.yaml    # Function metadata
│       │           └── handler.ts    # Implementation
│       ├── triggers/          # Event-driven handlers
│       │   └── on-{event}/
│       │       ├── .node.yaml
│       │       └── handler.ts
│       └── flows/             # Multi-step orchestration
│           └── {flow-name}/
│               └── .node.yaml       # Flow definition
│
├── static/                    # Static assets (images, CSS, JS)
│
└── .agent/                    # AI assistant context
    ├── context/               # Product and architecture context
    ├── domain/                # Domain-specific schemas and patterns
    ├── knowledge/             # Detailed guides (node-types, triggers, etc.)
    └── prompts/               # Prompt templates for common tasks
\`\`\`

## Key Directories

### nodetypes/
Each YAML file defines a data schema. The filename must match the NodeType name
(e.g. \`myapp:Article.yaml\`). Properties declare types like String, Int, Boolean,
DateTime, Reference, Section, and more.

### mixins/
Each YAML file defines a reusable set of properties that can be composed into
multiple NodeTypes. Mixins avoid duplicating common property groups (e.g. SEO
fields, timestamps, social metadata) across types. Mixins are installed before
NodeTypes since NodeTypes may reference them.

### archetypes/
Archetypes are page templates that bridge the data schema and the UI. They link to a
base NodeType, define which fields appear in the admin editor (with UI controls like
TextField, SectionField, CompositeField), specify which ElementTypes can be placed in
sections, and determine which frontend page component renders the content. Multiple
archetypes can share one NodeType for different layouts.

### workspaces/
Workspace YAML files declare allowed node types, root structure, and access rules.
Content in \`content/{workspace}/\` is provisioned when the workspace is created.

### content/functions/
Server-side code that runs inside RaisinDB. Library functions provide reusable
logic. Triggers react to content events. Flows orchestrate multi-step processes.

### static/
Files served as-is (images, stylesheets, client-side scripts). Referenced by
content nodes via path.
`;
}

export function workflowsMd(): string {
  return `# Development Workflows

## Create a Node Type

1. Design the schema -- decide on properties, types, and indexes
2. Create \`nodetypes/{namespace}:{Name}.yaml\`
3. Optionally create an archetype in \`archetypes/\` for editor support
4. Add the NodeType name to \`manifest.yaml\` under \`provides.nodetypes\`
5. Add to workspace \`allowed_node_types\` if content should be editable there
6. Validate: \`raisindb package create --check .\`

See \`AGENTS/tasks/create-node-type.md\` for detailed steps.

## Create a Mixin

1. Identify a set of properties shared across multiple NodeTypes
2. Create \`mixins/{namespace}:{Name}.yaml\` with the shared properties
3. Add the Mixin name to \`manifest.yaml\` under \`provides.mixins\`
4. Reference the mixin from NodeTypes that should include its properties
5. Validate: \`raisindb package create --check .\`

## Add a Trigger

1. Create a directory under \`content/functions/triggers/on-{event}/\`
2. Add \`.node.yaml\` with trigger configuration (event type, filter)
3. Write \`handler.ts\` with the event handler logic
4. Add the trigger path to \`manifest.yaml\` under \`provides.triggers\`
5. Validate: \`raisindb package create --check .\`

## Create a Library Function

1. Create a directory under \`content/functions/lib/{namespace}/{fn-name}/\`
2. Add \`.node.yaml\` with function metadata (name, parameters)
3. Write \`handler.ts\` implementing the function logic
4. Add the function path to \`manifest.yaml\` under \`provides.functions\`
5. Validate: \`raisindb package create --check .\`

## Validate Package

Always validate before uploading:

\`\`\`bash
raisindb package create --check .
\`\`\`

This checks:
- manifest.yaml is valid and complete
- All referenced NodeTypes, Archetypes, ElementTypes exist
- Workspace definitions are consistent
- Function and trigger configurations are valid
- Content nodes conform to their NodeType schemas

## Build and Upload

\`\`\`bash
# Build the .rap archive
raisindb package create .

# Upload to server
raisindb package upload {packageName}-0.1.0.rap
\`\`\`

## Live Development with Sync

During development, use sync for instant feedback:

\`\`\`bash
raisindb package sync .
\`\`\`

This watches for file changes and pushes updates to the server without
rebuilding the full package. Faster iteration than build + upload.
`;
}
