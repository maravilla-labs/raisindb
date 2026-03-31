# Package Directory Structure

```
package/
├── manifest.yaml              # Package metadata, version, provides list
├── README.md                  # Package documentation
│
├── nodetypes/                 # Data schemas
│   └── {namespace}:{Name}.yaml  # One file per NodeType
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
```

## Key Directories

### nodetypes/
Each YAML file defines a data schema. The filename must match the NodeType name
(e.g. `myapp:Article.yaml`). Properties declare types like String, Int, Boolean,
DateTime, Reference, Section, and more.

### archetypes/
Archetypes are page templates that bridge the data schema and the UI. They link to a
base NodeType, define which fields appear in the admin editor (with UI controls like
TextField, SectionField, CompositeField), specify which ElementTypes can be placed in
sections, and determine which frontend page component renders the content. Multiple
archetypes can share one NodeType for different layouts.

### workspaces/
Workspace YAML files declare allowed node types, root structure, and access rules.
Content in `content/{workspace}/` is provisioned when the workspace is created.

### content/functions/
Server-side code that runs inside RaisinDB. Library functions provide reusable
logic. Triggers react to content events. Flows orchestrate multi-step processes.

### static/
Files served as-is (images, stylesheets, client-side scripts). Referenced by
content nodes via path.
