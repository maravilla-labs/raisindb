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
