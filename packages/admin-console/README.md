# RaisinDB Admin Console

A modern, glassmorphism-styled web administration interface for RaisinDB.

## License

This project is licensed under the Business Source License 1.1 (BSL-1.1). See the LICENSE file in the repository root for details.

## Features

### Content Management
- **Content Explorer**: Tree-based workspace content navigation with revision browser
- **Workspaces**: Create, configure, and manage content workspaces
- **Branch Management**: Git-like branching, merging, and tagging operations
- **Revision History**: Version history viewer with diff comparison
- **Global Search**: Full-text search across all content

### Schema Definition
- **Node Type Editor**: IDE-style editor with YAML + visual builder
- **Archetypes**: Base type definitions with inheritance
- **Element Types**: Element type definitions
- **Relation Types**: Relationship type management

### Access Control
- **Users**: Repository-level user management
- **Roles**: Role-based access control (RBAC)
- **Groups**: User group management
- **Entity Circles**: Permission circles for fine-grained access control
- **Admin Users**: System-level administrator management
- **Identity Users**: Identity provider integration (OAuth/SSO)

### Development Tools
- **SQL Query IDE**: SQL/Cypher editor with execution history and job monitoring
- **Functions IDE**: VS Code-like editor for serverless functions
- **Flows**: Workflow management and execution monitoring
- **Agents**: Create and manage intelligent agents
- **Packages**: Export and import content packages

### System Management
- **Database Management**: RocksDB operations and configuration
- **Jobs Management**: Job queue monitoring and execution
- **Execution Logs**: Detailed operation and execution logging
- **System Health**: Real-time health monitoring via SSE
- **Metrics Dashboard**: Performance metrics visualization
- **AI Settings**: Embedding model configuration (Hugging Face integration)
- **Auth Settings**: Authentication provider configuration

### Modern UI
- Glassmorphism design with TailwindCSS 4
- Dark mode with purple/blue gradients
- Responsive and accessible
- Lucide icons

## Tech Stack

- **Frontend**: React 18.3 + TypeScript 5.7
- **Styling**: TailwindCSS 4 (zero-config with @tailwindcss/vite)
- **Build Tool**: Vite 6
- **Code Editor**: Monaco Editor (VS Code's editor)
- **Router**: React Router 7
- **Icons**: Lucide React
- **Drag & Drop**: Pragmatic Drag and Drop (Atlassian)
- **Panels**: Allotment for resizable splits
- **Backend Integration**: Embedded in Rust binary via `rust-embed`

## Internal Dependencies

This package depends on other packages within the RaisinDB monorepo:

| Package | Description |
|---------|-------------|
| `@raisindb/editor` | Shared editor components (`../raisin-editor`) |
| `@raisindb/flow-designer` | Visual flow/workflow designer (`../raisin-flow-designer`) |
| `@raisindb/sql-wasm` | WASM-based SQL validation (`../../tooling/packages/raisin-sql-wasm`) |
| `raisin-rel-wasm` | Raisin Expression Language (REL) WASM bindings for validation and autocomplete (`../../tooling/packages/raisin-rel-wasm`) |

## Development

### Prerequisites

- Node.js 18+ and npm
- Rust toolchain (for WASM builds and embedding)

### Local Development

```bash
# Install dependencies
cd packages/admin-console
npm install

# Build WASM dependencies (required first time)
npm run prebuild

# Start dev server (with HMR)
npm run dev

# Make sure raisin-server is running on port 8081 for API proxying
```

Visit `http://localhost:5173` for the dev server with hot module replacement.

### Building

```bash
# Build for production (includes TypeScript checking)
npm run build

# Output goes to ../../crates/raisin-server/.admin-console-dist
```

The build is automatically triggered when running `cargo build` on the `raisin-server` crate via `build.rs`.

### Linting

```bash
npm run lint
```

## Architecture

### Project Structure

```
src/
├── api/           # API client modules for server communication
├── components/    # Reusable React components
│   ├── PropertyFields/      # Type-specific property editors
│   ├── archetype-builder/   # Archetype visual builder
│   ├── element-builder/     # Element type builder
│   ├── nodetype-builder/    # Node type visual builder
│   ├── graph/               # Graph visualization components
│   ├── management/          # System management components
│   └── shared/              # Shared UI components
├── contexts/      # React Context providers (AuthContext)
├── hooks/         # Custom React hooks
├── monaco/        # Monaco Editor integration (SQL, DDL)
├── pages/         # Route page components
│   ├── agents/              # Agent management pages
│   ├── functions/           # Functions IDE pages
│   ├── management/          # System management pages
│   └── packages/            # Package management pages
├── utils/         # Utility functions
├── generated/     # Auto-generated types
├── assets/        # Static assets
└── styles/        # Global CSS
```

### Routing

The admin console uses React Router with repository-scoped and tenant-level routes:

**Entry Points:**
- `/admin` - Repository list (entry point)
- `/admin/login` - Authentication

**Repository-Scoped Routes** (`/:repo`):
- `/content` - Workspace selector
- `/content/:branch/:workspace/*` - Content explorer
- `/workspaces` - Workspace management
- `/nodetypes`, `/:branch/nodetypes` - Node type management
- `/archetypes`, `/:branch/archetypes` - Archetype management
- `/elementtypes`, `/:branch/elementtypes` - Element type management
- `/users`, `/roles`, `/groups`, `/circles` - Access control
- `/functions`, `/functions/:branch/*` - Functions IDE
- `/agents`, `/:branch/agents` - Agent management
- `/packages/*` - Package management
- `/query` - SQL Query IDE
- `/branches` - Branch management
- `/flows` - Flow management
- `/logs` - Execution logs

**Tenant-Level Routes** (`/management`):
- `/database` - Database management
- `/ai` - AI/embedding settings
- `/auth` - Authentication settings
- `/rocksdb` - RocksDB management
- `/jobs` - Job queue management
- `/logs` - Execution logs
- `/flows` - Flow execution monitor
- `/admin-users` - Admin user management
- `/identity-users` - Identity provider users
- `/profile` - User profile

### API Integration

The console communicates with RaisinDB's REST API through modules in `src/api/`:

- **Content**: `nodes.ts`, `workspaces.ts`, `repositories.ts`
- **Schema**: `nodetypes.ts`, `archetypes.ts`, `elementtypes.ts`
- **Query**: `sql.ts`, `search.ts`
- **Versioning**: `branches.ts`, `revisions.ts`, `translations.ts`
- **Access Control**: `users.ts`, `roles.ts`, `groups.ts`, `admin-users.ts`, `identity-users.ts`
- **Functions**: `functions.ts`, `flows.ts`, `agents.ts`
- **System**: `management.ts`, `ai.ts`, `processing-rules.ts`, `packages.ts`, `jobs.ts`
- **Auth**: `auth.ts`, `identity-auth.ts`, `api-keys.ts`

### YAML-First Approach

Node types are created and edited as YAML documents:

```yaml
name: my:ContentType
extends: raisin:Page
properties:
  title:
    type: String
    required: true
  author:
    type: String
    required: false
allowed_children:
  - my:Article
  - raisin:Folder
```

The editor provides:
- Syntax highlighting
- Auto-completion
- Real-time validation
- Format on save
- Resolved view showing full inheritance chain
- Visual builder alternative

## Deployment

The admin console is embedded in the `raisin-server` binary at compile time via `rust-embed`. No separate deployment needed.

Access the console at: `http://localhost:8080/admin`

## Design Philosophy

1. **Content-first**: Focus on managing content structures and workflows
2. **Tree-based navigation**: Hierarchical content browsing with revision history
3. **Developer-friendly**: YAML editing, SQL IDE, Monaco editor integration
4. **Modern aesthetics**: Glassmorphism, smooth animations, dark mode
5. **Embedded deployment**: Single binary, no separate frontend server
6. **Real-time feedback**: SSE-based health monitoring and live updates
