# System Definitions & Updates

RaisinDB ships a set of built-in definitions — `raisin:*` NodeTypes, global
Workspaces, and builtin packages. This document covers where they come from,
how a change reaches an existing repository, and how to ship a change **without
a new server release**.

## The two halves

| | What it controls | Surface |
|---|---|---|
| **System definitions** | what this *server* offers | `/api/management/system-definitions*` |
| **System updates** | what a *repository* has applied | `/api/management/repositories/{tenant}/{repo}/system-updates*` |

They are deliberately separate. Changing a definition on the server never
rewrites tenant schemas by itself; a repository picks the change up either
through the startup resync (non-breaking only, by default) or through an
explicit apply.

## Layers

Definitions resolve through a stack, lowest precedence first:

| Layer | Source | Overridable |
|---|---|---|
| `embedded` | compiled into the binary (`include_dir!`) | always present, always the baseline |
| `overlay` | a directory on the server | yes — overrides `embedded` by name |
| registry cache | artifacts fetched from a registry **into** the overlay dir | same as overlay |

Resolution is by resource **name**, whole-definition: the highest layer that
defines a name wins outright. There is no field-level merging — a
half-overridden schema is one nobody wrote.

A resolved definition carries its winning layer's content hash, so an overlay
edit shows up in the pending-updates view exactly like a definition shipped in a
new binary.

Code: `crates/raisin-core/src/definitions/`.

## Configuration

```toml
[system_definitions]
# off | non_breaking | all   (default: non_breaking)
auto_apply = "non_breaking"

# Default: <data_dir>/system-definitions — a missing directory is fine.
overlay_dir = "/data/raisindb/system-definitions"

# Optional, opt-in, never contacted unless enabled.
[[system_definitions.registries]]
name    = "public"
url     = "https://example.com/raisindb-definitions/index.json"
enabled = false
# token = "env:MY_REGISTRY_TOKEN"
```

Every default reproduces the behaviour of a server with no such section:
embedded definitions only, no registries, non-breaking changes applied on start.

## Content hash, not `version:`

The startup resync used to write a global NodeType into an existing repository
only when the YAML's `version:` integer had been bumped. Editing a definition
and forgetting that bump was a **silent** no-op for every pre-existing tenant —
new repos got the new schema, old repos kept the old one and rejected writes
carrying the new properties.

The resync now compares **content hashes**. Any edit propagates, bump or no
bump. Safety comes from classification instead:

- **Non-breaking** (new property, relaxed constraint, …) → applied automatically
  under the default `auto_apply = "non_breaking"`.
- **Breaking** (property removed, type changed, `required` added, allowed child
  or mixin removed) → withheld, logged, and listed as a pending update for an
  explicit apply with `force: true`.

`version:` is still useful documentation of a schema change, but it is no longer
load-bearing.

### Workspaces: allow-lists are merged, not replaced

Builtin packages **extend** a workspace's `allowed_node_types` /
`allowed_root_node_types` when they install (`workspace_patches` in a package
manifest — e.g. `ai-tools` adds `raisin:AIAgent` to `functions`). The workspace
YAML is therefore only the *base*, never the whole picture.

The unattended resync unions the two lists rather than replacing them, so new
entries from the YAML land while package-contributed entries survive. Without
that, every server would strip its packages' node types — or, because a removal
counts as breaking, park `default`, `functions` and `raisin:access_control` in
the pending list permanently on a brand-new install.

The explicit admin apply still replaces wholesale: removing an entry is a
deliberate act and stays an operator decision.

Code: `crates/raisin-core/src/system_updates/resync.rs`,
`crates/raisin-server/src/startup/binary.rs`.

## Shipping a fix without a release

The overlay directory mirrors the source tree, so a corrected definition can be
copied straight out of a checkout:

```text
<overlay_dir>/
  nodetypes/raisin_package.yaml       # like crates/raisin-core/global_nodetypes/
  workspaces/content.yaml             # like crates/raisin-core/global_workspaces/
  packages/raisin-auth/manifest.yaml  # like builtin-packages/<name>/
```

Then, with no restart:

```bash
# 1. Land the file, then make the server re-read the overlay
curl -X POST .../api/management/system-definitions/reload

# 2. See what changed and where each definition now resolves from
curl .../api/management/system-definitions

# 3. Per repository: review pending updates, then apply
curl .../api/management/repositories/$TENANT/$REPO/system-updates
curl -X POST .../api/management/repositories/$TENANT/$REPO/system-updates/apply \
     -d '{"resources":["raisin:Package"]}'
```

A malformed overlay file is skipped with an error in the log and the embedded
definition stays in force — a bad file cannot brick the server or silently
shadow a good built-in.

The admin console surfaces all of this on the **System Updates** page
(`Definition Sources` panel): current layers, the overlay path, which
definitions are overridden, a reload button, and registry browsing.

## Registries (optional)

A registry is an HTTP-reachable index of definitions and packages. It is
**opt-in and never automatic** — nothing is fetched on startup, on a timer, or
as a side effect of a read. A fetch happens only when an operator asks for one,
lands the artifact in the overlay directory, and stops there; applying it to a
repository is still the separate system-updates step.

Index format:

```json
{
  "schema": 1,
  "entries": [
    { "name": "raisin:Package", "kind": "nodetype", "version": "2",
      "sha256": "…", "url": "https://…/raisin_package.yaml" },
    { "name": "raisin-auth", "kind": "package", "version": "0.3.0",
      "sha256": "…", "url": "https://…/raisin-auth.rap" }
  ]
}
```

`kind` is `nodetype` (alias `node_type`), `workspace`, or `package`.

**Trust model.** Every download is checked against the `sha256` the index
declares and rejected on mismatch. That covers corruption and a swapped artifact
behind a stable URL — it does **not** authenticate the index itself. There is no
artifact signing today; trust rests on the operator having configured a URL they
trust, plus TLS. Treat a registry URL with the same care as a package you
install by hand. Multiple registries are allowed and no URL is hardcoded
anywhere in the server, so a deployment can point at a public index, a private
mirror, or nothing at all.

**Status: unexercised.** The fetch path has never run against a real index —
`RegistryClient::fetch_entries` has one caller (the HTTP handler) and no test,
and `unpack_rap` has never executed even in a test. The registry tests cover
index parsing, the disabled-registry guard, and filename sanitisation only.
Treat this path as unproven until someone stands up an index and does one
end-to-end fetch of a real `.rap`.

## Clusters: read this before enabling replication

**The single-node behaviour is correct. Multi-node is NOT yet safe** — not
because data corrupts (CRDT converges), but because the distribution model has
three holes. None of this is live while a deployment runs one node.

The intuition "one node downloads it, writes it into the repository, and
replication installs it everywhere" is **half right**:

| Layer | Replicates? |
|---|---|
| NodeType / Workspace definitions | **Yes** — written through storage, captured as `OpType::UpdateNodeType` / `UpdateWorkspace` |
| The `raisin:Package` node | **Yes** — an ordinary node |
| Package *content* installed by the job | **Yes** — ordinary nodes, so install-once-then-replicate works |
| The `.rap` archive bytes | **No** (filesystem backend) |
| The overlay directory | **No** — local disk on whichever node fetched |
| The applied-hash ledger | **No** |

### 1. The `.rap` bytes do not replicate

`apply_package_update` stores the archive through `BinaryStorage::put_bytes`.
With the **filesystem** backend those bytes exist only on the node that fetched
them. The Package node replicates and points at a `resource.key`, but on every
other node that key resolves to nothing — so a later on-demand install of an
`auto_install: false` package fails there.

Fix: run the **`s3` binary backend**, which makes the blob genuinely shared.

### 2. The applied-hash ledger is node-local

`SystemUpdateRepositoryImpl::set_applied`
(`raisin-rocksdb/src/repositories/system_updates.rs`) is a raw `db.put_cf` on
the `SYSTEM_UPDATE_HASHES` column family — no replication capture, no
transaction. Every node keeps its own opinion of what has been applied, so on a
cluster restart all N nodes independently resync and all N write identical
definitions: N× replication ops at boot. Converges, but wasteful.

### 3. The real hazard: silent cluster-wide revert

Node A has an overlay definition and applies it; it replicates, so storage is
correct everywhere. But B's *ledger* still records the embedded hash and B's
resolver still serves embedded — ledger and storage now disagree on B. The next
time B's embedded hash changes (any upgrade), B's resync compares against its
own ledger, sees a mismatch, applies **embedded**, and replicates that over A's
overlay version. The overlay reverts across the whole cluster, silently.

This is the same clobber class as the in-process one that
[`crate::definitions::global_resolver`] exists to prevent, reappearing between
nodes. Nothing guards it today: the resync takes no lock, and the `inprocess`
lock backend is single-node only regardless.

### Making it safe

In increasing order of effort:

1. **Distribute the overlay as configuration.** Have config management push the
   overlay directory to *every* node, so all nodes resolve identically and the
   node-local ledger is consistent by construction. Works today with no code
   change — but it is "config management distributes", not "one node downloads".
2. **Replicate `SYSTEM_UPDATE_HASHES` and use the `s3` binary backend.**
   Together these fix (1), (2) and (3) and make fetch-on-any-node correct — the
   model most people expect.
3. **Guard the startup resync with a cluster lock** (`raisin-locks` with the
   `redis` backend) so exactly one node applies per cluster.

## Endpoints

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/management/system-definitions` | layers, overlay path, per-definition origin |
| `POST` | `/api/management/system-definitions/reload` | re-read the overlay directory |
| `GET` | `/api/management/system-definitions/registries` | configured registries |
| `GET` | `/api/management/system-definitions/registries/{name}` | that registry's catalog |
| `POST` | `/api/management/system-definitions/registries/{name}/fetch` | download artifacts into the overlay |
| `GET` | `/api/management/repositories/{tenant}/{repo}/system-updates` | pending updates for a repo |
| `POST` | `/api/management/repositories/{tenant}/{repo}/system-updates/apply` | apply them (`force` for breaking) |
