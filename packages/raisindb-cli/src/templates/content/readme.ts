import type { TemplateVars } from '../types.js';

export function rootReadme(vars: TemplateVars): string {
  return `# {{packageName}}

{{description}}

This project was bootstrapped with \`raisindb package init\`.

## Project Structure

\`\`\`
{{packageName}}/
├── package/       # RaisinDB content package (schemas, content, functions)
└── frontend/      # Your application (web app, mobile app, etc.)
\`\`\`

## Getting Started

### Prerequisites

- [RaisinDB](https://raisindb.com) server running locally
- \`raisindb\` CLI installed (\`npm install -g @raisindb/cli\`)

### 1. Start your RaisinDB server

\`\`\`bash
raisindb
\`\`\`

### 2. Validate and upload the content package

\`\`\`bash
cd package
raisindb package create --check .   # Validate schemas
raisindb package create .            # Build .rap archive
raisindb package upload {{packageName}}-0.1.0.rap
\`\`\`

### 3. Build your frontend

The \`frontend/\` directory is where your application code lives. Connect to
RaisinDB using the \`@raisindb/client\` SDK:

\`\`\`bash
cd frontend
# Set up your framework of choice (React, Svelte, Vue, etc.)
npm install @raisindb/client
\`\`\`

## Available Scripts

### Package commands (run from \`package/\`)

| Command | Description |
|---------|-------------|
| \`raisindb package create --check .\` | Validate package structure |
| \`raisindb package create .\` | Build \`.rap\` archive |
| \`raisindb package upload <file>.rap\` | Deploy to server |
| \`raisindb package sync .\` | Live sync during development |

## Learn More

- \`package/.agent/knowledge/\` -- Detailed guides on RaisinDB concepts
- \`package/AGENT.md\` -- AI coding agent instructions
`;
}

export function packageReadme(vars: TemplateVars): string {
  return `# {{packageName}} -- Content Package

This directory contains the RaisinDB content package: data schemas, content,
server-side functions, and triggers.

## Package Commands

\`\`\`bash
# Validate package structure
raisindb package create --check .

# Build .rap package
raisindb package create .

# Upload to server
raisindb package upload {{packageName}}-0.1.0.rap

# Live sync during development
raisindb package sync .
\`\`\`

## Directory Structure

\`\`\`
package/
├── manifest.yaml           # Package metadata
├── workspaces/
│   └── {{workspace}}.yaml  # Workspace definition
├── nodetypes/              # Data schemas (NodeType YAML)
├── archetypes/             # Page templates (editor + frontend component mapping)
├── elementtypes/           # Composable content blocks (frontend components)
├── content/
│   ├── {{workspace}}/      # Initial content for workspace
│   └── functions/          # Server-side functions & triggers
└── static/                 # Static assets
\`\`\`

## Content Model

- **NodeType** -- data schema (like a database table)
- **Archetype** -- page template linking a NodeType to editor fields and a frontend component
- **ElementType** -- composable content block placed in SectionFields, mapped to a frontend component
- **Workspace** -- isolated content space with allowed node types

## Learn More

See \`.agent/knowledge/\` for detailed guides on RaisinDB concepts.
`;
}

export function frontendReadme(vars: TemplateVars): string {
  return `# {{packageName}} -- Frontend

This directory is the placeholder for your application code.

## Getting Started

Set up your framework of choice and install the RaisinDB client SDK:

\`\`\`bash
# Example: Create a React app
npx create-react-app . --template typescript

# Or Svelte
npm create svelte@latest .

# Then install the RaisinDB client
npm install @raisindb/client
\`\`\`

## Connecting to RaisinDB

\`\`\`typescript
import { createClient } from '@raisindb/client';

const client = createClient('raisin://localhost:8081/default/{{packageName}}');

// Query content
const db = client.database('{{packageName}}');
const result = await db.sql\`SELECT * FROM '{{workspace}}' WHERE node_type = 'raisin:Folder'\`;
\`\`\`

## Learn More

See \`../package/.agent/knowledge/sdk/\` for client SDK documentation.
`;
}

