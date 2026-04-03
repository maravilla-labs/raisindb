# RaisinDB Agent Skills

Reusable AI agent skills for building RaisinDB applications. Works with Claude Code, Cursor, GitHub Copilot, and 30+ other AI coding tools.

## Install

```bash
npx skills add maravilla-labs/raisindb/packages/raisindb-skills
```

## Skills

| Skill | Description |
|-------|-------------|
| **raisindb-overview** | Core concepts: path-as-URL routing, archetype-to-component mapping, project structure |
| **raisindb-content-modeling** | Define NodeTypes, Archetypes, ElementTypes in YAML |
| **raisindb-frontend-sveltekit** | SvelteKit frontend with dynamic routing and component registries |
| **raisindb-frontend-react** | React Router frontend with SSR-to-WebSocket upgrade |
| **raisindb-sql** | SQL syntax: CRUD, JSONB, hierarchy queries, graph relations |
| **raisindb-translations** | Multi-language content with `.node.{locale}.yaml` files |
| **raisindb-auth** | Authentication: anonymous, login, register, session management |
| **raisindb-file-uploads** | File uploads, asset management, signed URLs |
| **raisindb-access-control** | Roles, permissions, groups, row-level security |
| **raisindb-functions-triggers** | Server-side JavaScript functions and event-driven triggers |

## Learning Path

1. **Start here** → `raisindb-overview`
2. **Model your data** → `raisindb-content-modeling`
3. **Build frontend** → `raisindb-frontend-sveltekit` or `raisindb-frontend-react`
4. **Query data** → `raisindb-sql`

Then as needed: `translations`, `auth`, `file-uploads`, `functions-triggers`, `access-control`
