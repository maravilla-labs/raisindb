# Key Technical Decisions

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
