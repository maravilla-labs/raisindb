---
sidebar_position: 1
---

# Examples & Tutorials

These samples mirror the repositories included in this repo so you can run them locally and map each action to the real APIs.

## Web App Examples

| Path | Highlights |
|------|------------|
| `packages/raisin-client-js/examples/social-feed` | Browser SPA that uses the official JS client, WebSockets, and REST fallbacks |
| `packages/raisin-client-js/examples/social-feed-ssr` | Server-side rendered variant demonstrating Node.js WebSocket usage |

Both projects rely on the SDK modules in `packages/raisin-client-js/src` (`nodes.ts`, `node-types.ts`, `sql.ts`, etc.). Open them to see how each SDK call maps to REST endpoints such as `/api/repository/{repo}/{branch}/head/{ws}/...` and `/api/sql/{repo}`.

## Cluster Demos

- `examples/cluster/node1.toml` (plus node2/node3) spins up a 3-node replication topology.
- Each node surfaces the admin endpoints listed in [Operate RaisinDB](../operate/overview.md).

## Query Playgrounds

- `test_query.sql`, `test_query.sql` – SQL snippets that exercise RaisinSQL features highlighted in [Access → SQL](../access/sql/overview.md).
- `sqloutput.txt` – sample outputs captured from the real planner/executor.

## Scripts

- `start-cluster.sh` – helper for booting multiple servers with RocksDB.
- `test_branch_tag_api.sh`, `test_children_snapshot.rs` – regression suites covering branch/tag/APIs and tree snapshots.

## How to Use These Assets

1. Run `cargo test` or individual scripts to validate a feature.
2. Cross-reference the API docs whenever you touch a handler or SDK.
3. Promote working demos into tutorials by copying the exact source file into this documentation.

Because every sample lives in the repository, you can trust that it reflects the current implementation. Update the docs in lockstep whenever you modify the associated code.
