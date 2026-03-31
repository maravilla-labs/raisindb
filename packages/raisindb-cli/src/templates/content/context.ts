import type { TemplateVars } from '../types.js';

export function productContext(vars: TemplateVars): string {
  return `# {{packageName}}

## What This Package Does

{{description}}

## Target Users

<!-- Who is this package for? -->

## Key Features

<!-- List the main capabilities this package provides -->
-

## Domain Concepts

<!-- What domain-specific terms or concepts should an AI assistant understand? -->
-
`;
}

export function architectureContext(): string {
  return `# RaisinDB Package Architecture

## Content-Driven Design

RaisinDB packages follow a content-driven architecture where everything is modeled
as content nodes stored in a structured database. Packages define schemas, content,
and server-side logic that get deployed together.

## Core Concepts

### Workspaces

A workspace is an isolated content space. Each workspace defines which node types
are allowed, what the root structure looks like, and who has access. Packages can
provision one or more workspaces with pre-defined content.

### NodeTypes

NodeTypes are data schemas that define what properties content CAN have. They declare
typed properties (String, Number, Boolean, Date, Object, Array, Reference, Resource)
and are referenced by name with a namespace prefix (e.g. \`myapp:Article\`).

### Mixins

Mixins are reusable property sets (NodeTypes with \`is_mixin: true\`) that can be
composed into multiple NodeTypes. Define common property groups (SEO fields, audit
trails, social metadata) once as a mixin and include them in any NodeType. In SQL,
use \`CREATE MIXIN\`, \`ALTER MIXIN\`, and \`DROP MIXIN\` statements. In packages, mixins
live in the \`mixins/\` directory and are installed before NodeTypes.

### Archetypes

Archetypes are page templates that sit between the data schema and the UI. An archetype
links to a base NodeType and defines: (1) which fields appear in the admin editor with
what UI controls, (2) which ElementTypes can be placed in SectionFields, and (3) which
frontend page component renders this content. Multiple archetypes can share the same
base NodeType for different page layouts. The frontend maps archetype names to
components: \`pageComponents['myapp:BlogPost'] = BlogPostPage\`.

### ElementTypes

ElementTypes are composable content blocks placed inside SectionFields of archetypes.
Each element type defines its own fields (headline, body, image, etc.) and maps to a
frontend component. A page's \`content\` property is an array of elements, each with an
\`element_type\` and \`uuid\`. The frontend renders them by looking up
\`elementComponents[element.element_type]\`.

### Functions

Server-side functions run inside RaisinDB. They can be:
- **Library functions** (\`content/functions/lib/\`) -- reusable logic callable from triggers and flows
- **Trigger handlers** (\`content/functions/triggers/\`) -- event listeners that react to content changes

### Triggers

Triggers subscribe to content events (node created, updated, deleted) and invoke
functions in response. They enable event-driven automation without polling.

### Flows

Flows orchestrate multi-step processes. They can chain function calls, wait for
human input, call AI providers, and manage long-running workflows with
compensation (rollback) support.

## Package Lifecycle

1. **Author** -- define schemas, content, and functions locally
2. **Validate** -- run \`raisindb package create --check .\` to catch errors early
3. **Build** -- run \`raisindb package create .\` to produce a \`.rap\` archive
4. **Upload** -- deploy with \`raisindb package upload <file>.rap\`
5. **Sync** -- use \`raisindb package sync .\` during development for live reload
`;
}

export function decisionsContext(): string {
  return `# Key Technical Decisions

Track important decisions made during development so the team (and AI assistants)
understand why things are the way they are.

## Template

### Decision: [Short title]
- **Date**: YYYY-MM-DD
- **Status**: Accepted | Superseded | Deprecated
- **Context**: What prompted this decision?
- **Decision**: What was decided?
- **Consequences**: What are the trade-offs?

---

## Decisions

### Decision: Use RaisinDB content-driven architecture
- **Date**: (project start)
- **Status**: Accepted
- **Context**: Needed a structured content backend with real-time sync and schema validation.
- **Decision**: Build on RaisinDB with NodeType schemas and workspace isolation.
- **Consequences**: Content is strongly typed and validated. Schema changes require NodeType updates.

<!-- Add more decisions as the project evolves -->
`;
}
