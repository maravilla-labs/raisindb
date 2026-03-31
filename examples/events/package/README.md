# events -- Content Package

This directory contains the RaisinDB content package: data schemas, content,
server-side functions, and triggers.

## Package Commands

```bash
# Validate package structure
raisindb package create --check .

# Build .rap package
raisindb package create .

# Upload to server
raisindb package upload events-0.1.0.rap

# Live sync during development
raisindb package sync .
```

## Directory Structure

```
package/
├── manifest.yaml           # Package metadata
├── workspaces/
│   └── events.yaml  # Workspace definition
├── nodetypes/              # Data schemas (NodeType YAML)
├── archetypes/             # Page templates (editor + frontend component mapping)
├── elementtypes/           # Composable content blocks (frontend components)
├── content/
│   ├── events/      # Initial content for workspace
│   └── functions/          # Server-side functions & triggers
└── static/                 # Static assets
```

## Content Model

- **NodeType** -- data schema (like a database table)
- **Archetype** -- page template linking a NodeType to editor fields and a frontend component
- **ElementType** -- composable content block placed in SectionFields, mapped to a frontend component
- **Workspace** -- isolated content space with allowed node types

## Learn More

See `.agent/knowledge/` for detailed guides on RaisinDB concepts.
