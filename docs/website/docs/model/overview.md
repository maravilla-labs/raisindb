---
sidebar_position: 1
---

# Model Your Data

RaisinDB enforces structure through NodeTypes, workspaces, and repositories. All behavior described here is implemented inside the `NodeService` and related helpers in `crates/raisin-core`.

## Building Blocks

| Concept | Backed by | Notes |
|---------|-----------|-------|
| Repository | `WorkspaceService::repository` | Isolation boundary similar to a database |
| Workspace | `NodeService::new_with_context` | Contains tree-structured nodes |
| NodeType | `crates/raisin-models` schemas + validation | YAML/JSON definition shared across workspaces |
| Revision | `raisin_hlc::HLC` | Each commit receives a Hybrid Logical Clock value |

## Workspaces and Branches

- **Branches**: HEAD vs revision routing in `raisin-transport-http/src/routes.rs` exposes mutable and historical views.
- **Workspaces**: Initialization happens through the workspace handlers, and every node request includes `{repo}/{branch}/{ws}` to target the correct tree.
- **Draft vs committed data**: `NodeService::overlay_workspace_deltas` merges uncommitted workspace deltas with committed nodes, so your draft edits live alongside published content without conflicts.

## NodeTypes

NodeTypes define validation and indexing rules. They are stored in the management handlers under `/api/management/{repo}/{branch}/nodetypes`.

```yaml
name: blog:Article
description: A blog post
properties:
  - name: title
    type: String
    required: true
    indexed_for_sql: true
    fulltext_indexed: true
  - name: body
    type: Text
    fulltext_indexed: true
  - name: status
    type: String
    indexed_for_sql: true
allowed_children: ["raisin:Asset"]
versionable: true
publishable: true
```

## Validation Pipeline

1. **Schema resolution** – the NodeType resolver merges inheritance before validation.
2. **Node validation** – `NodeValidator` (see `crates/raisin-core/src/services/node_validation.rs`) enforces required fields, enum values, and reference integrity.
3. **Audit hooks** – when enabled, `RepoAuditAdapter` logs actions to the audit repository.

## Versioning and Publication

- **Revision pinning** – call read endpoints under `/rev/{revision}` to time-travel using the encoded HLC value.
- **Publish/unpublish** – `copy_publish.rs` handles promotion from draft to public branches, mirroring Git workflows.
- **Branch merges** – the RocksDB-only merge endpoints orchestrate comparison and resolution using the transaction context in `crates/raisin-rocksdb/src/transaction`.

## Next Steps

- Define NodeTypes under [`model/nodetypes`](./nodetypes/overview.md).
- Explore SQL querying options under [`Access → SQL`](../access/sql/overview.md).
- Use the REST management APIs to automate repository bootstrapping.
