---
name: raisindb-branch-workflows
description: "Work with RaisinDB branches: fork an isolated branch, write to it, compare and merge back. Use when an agent needs an isolated workspace, when staging schema/content changes before main, or when deploying a package to a non-default branch."
---

# RaisinDB Branch Workflows

A branch is an isolated copy of **both schema and content**. Writes on one branch
are invisible on another until you merge. Use branches to give an AI agent its
own workspace and merge the result back, to stage schema/content changes before
promoting to `main`, or to run parallel experiments without touching production.

Branches are scoped per `{tenant, repo, branch}` — every node, index, archetype,
element type, and node type is keyed by branch.

## 1. Scope work to a branch — `onBranch`

`db.onBranch(name)` returns a branch-scoped database/workspace. The default
branch is `main`.

```ts
const db = client.database('myapp');

// Write on `staging` in isolation — main is untouched
const staging = db.onBranch('staging');
await staging.workspace('content').nodes().create({
  type: 'Article',
  path: '/articles/wip',
  properties: { title: 'Work in progress' },
});

// Reads/schema lookups on a branch
await db.onBranch('staging').archetypes().list();
```

For the HTTP client the branch is part of the REST path
(`/api/management/{repo}/{branch}/...` and `/api/repository/{repo}/{branch}/...`);
for the WebSocket client it travels in the request context. `onBranch('x')`
works on both.

## 2. Branch lifecycle — `db.branches()`

```ts
const branches = db.branches();

// Fork a FULL copy of main (nodes, indexes, AND schema) at its current HEAD
await branches.create('staging', { fromBranch: 'main' });

// Or fork at a specific revision
await branches.create('snapshot', { fromRevision: '<revision-id>' });

await branches.list();
await branches.get('staging');
await branches.getHead('main');
await branches.delete('staging');
```

**Forking copies the schema.** A `fromBranch` fork copies the archetype registry
(plus element types and node types), so writing archetyped nodes on the new
branch works immediately — you do **not** need to redeploy the schema to it.

> Pitfall: `create('x')` with **no** `fromBranch`/`fromRevision` makes an
> **empty** branch (no schema, no content). To get a usable copy, always pass
> `fromBranch` (or `fromRevision`).

## 3. Compare and merge

```ts
// Divergence (how far `staging` is ahead/behind `main`) — for a status/diff view
const divergence = await db.branches().compare('staging', 'main');

// Merge staging INTO main. Defaults to ThreeWay; fast-forwards when possible.
const result = await db.branches().merge('staging', 'main', {
  strategy: 'ThreeWay',          // or 'FastForward'
  message: 'Merge staging',
});
// result: { success, revision, fast_forward, conflicts, ... }
```

Strategies: `'ThreeWay'` (default, creates a merge commit) and `'FastForward'`.
On conflicts the merge returns them in `conflicts`; resolve via the HTTP
`resolve-merge` endpoint. **Merge and compare require the RocksDB backend (the
default).**

### Promote selected nodes — `copyNodes`

Merge takes a whole branch. When one branch holds work in progress and another
holds what is live, the usual need is to move *some* of it — a node and what it
depends on, not everything anyone has touched.

```ts
const at = await db.branches().getHead('main');   // decide once, copy that

await db.branches().copyNodes('main', 'live', {
  workspace: 'content',
  roots: ['/products/kettle'],
  recursive: true,          // default true
  deleteMissing: false,     // prune target nodes absent from the copied set
  sourceRevision: at,       // pin — see below (server 0.3.15+)
});
// { copied, deleted, revision, changes: [{ node_id, path, operation }] }
```

- **Ids are preserved.** Promoting the same root again updates the same target
  nodes, so repeats are idempotent for a given source state and references to a
  promoted node keep resolving. (`copy_node_tree` is the opposite — it mints new
  ids, for duplicating within one branch.)
- **The whole node travels**: properties, archetype, every index, branch-scoped
  secrets, and translation overlays, node-level and block-level.
- One atomic commit; the target HEAD advances once.
- Each root's parent path must already exist on the target, or the call fails
  rather than writing a dangling subtree.

**Pin it.** Copying a large set is not instantaneous. Without `sourceRevision`
the source is read at HEAD as each node's turn comes, so a write landing
mid-copy ends up partly inside the result — and nothing reports it, because
every node copied cleanly, just not all from the same moment. Capture the head
when the promotion is *decided* and pass it back. This is what makes "promote
what was reviewed" true when a review gate sits between the two.

The target is always resolved at its head: the pin says what to copy, never
where to put it.

### Agent-isolation pattern

```ts
// 1. Each agent gets its own branch off main
await db.branches().create(`agent/${agentId}`, { fromBranch: 'main' });

// 2. The agent reads/writes only through its branch
const work = db.onBranch(`agent/${agentId}`);
// ... agent does its work ...

// 3. Merge the agent's branch back into main when done
await db.branches().merge(`agent/${agentId}`, 'main', { message: `agent ${agentId}` });
```

## 4. Deploy a package to a branch — CLI `--branch`

`deploy`, `sync`, and `install` accept `-b, --branch <name>` (default `main`),
so you can push schema/content to a non-default branch:

```bash
# Upload + install a package onto the `staging` branch
raisindb deploy ./package --repo myapp --branch staging --install

# Live-sync edits against a branch
raisindb sync ./package --repo myapp --branch staging --watch --push

# Install an already-uploaded package on a branch
raisindb install my-package --repo myapp --branch staging
```

## 5. Cross-branch work inside a server-side function (SQL)

A server-side function/flow runs on a single fixed context branch, but raw
`raisin.sql` can target any branch per statement — so one function can read from
one branch and write to another. SQL branch targeting:

- **SELECT / UPDATE / DELETE** — `WHERE __branch = 'x'`
- **MOVE / COPY / ORDER / RELATE / UNRELATE / TRANSLATE** — `… IN BRANCH 'x'`
- **INSERT** — a `__branch` pseudo-column in the column list

```js
// In a function: write a node onto another branch.
raisin.sql.execute(
  "INSERT INTO stories (__branch, path, node_type, properties) \
   VALUES ('staging', $1, $2, $3)",
  [path, nodeType, JSON.stringify(props)]
);
```

:::danger Do not hand-roll a cross-branch COPY this way
Reading a node and re-INSERTing it on another branch looks equivalent to a
promotion and is not. Use `raisin.branches.copyNodes` instead. What the SQL
version silently gets wrong:

- **It copies only the columns you name.** Translation overlays live in their
  own keyspace and cannot be read from a function at all, so they are dropped —
  the copy succeeds and the other languages are simply not there.
- **It mints a new node id**, so the copy is a different entity from the source
  and anything referencing it by id stops resolving.
- **`SELECT`-then-`INSERT` can duplicate.** The existence check reads an
  eventually-consistent index, so a stale miss takes the INSERT branch and
  leaves TWO nodes at one path with different ids. No concurrency is needed —
  one re-run is enough.
- It cannot be pinned to a revision.

Raw `__branch` INSERT is right for writing *new* data to another branch. It is
not a copy.
:::

`__branch` on INSERT must be a string literal, the same for all rows, with an
explicit column list. It applies in auto-commit mode (each `raisin.sql` call);
inside `BEGIN … COMMIT` the branch is fixed at `BEGIN`. Note: the structured
`raisin.nodes.*` API is always single-branch (context branch) — use raw
`raisin.sql` for cross-branch writes.

## Quick reference

| Operation | Call |
|-----------|------|
| Scope reads/writes to a branch | `db.onBranch(name)` |
| Fork a usable branch | `db.branches().create(name, { fromBranch: 'main' })` |
| List / get / delete | `db.branches().list()` · `.get(name)` · `.delete(name)` |
| HEAD revision | `db.branches().getHead(name)` · `.updateHead(name, rev)` |
| Divergence (status/diff) | `db.branches().compare(branch, base)` |
| Merge | `db.branches().merge(source, target, { strategy, message })` |
| Promote selected nodes | `db.branches().copyNodes(source, target, { workspace, roots, sourceRevision })` |
| Deploy/sync/install to a branch | `raisindb deploy\|sync\|install ... --branch <name>` |
| Cross-branch write of NEW data in a function | raw SQL: `INSERT INTO ws (__branch, …) VALUES ('x', …)` |
| Cross-branch COPY of existing nodes | `raisin.branches.copyNodes(...)` — never SELECT-then-INSERT |
