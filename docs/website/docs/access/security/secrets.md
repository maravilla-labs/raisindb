---
sidebar_position: 4
draft: true
---

<!--
DRAFT — excluded from production builds by `draft: true`.

The store, the `encrypted: true` schema flag and the crypto layer are built. The
CLI commands, the `/api/secrets` endpoints and the `raisin.secrets` function
binding described below are NOT yet implemented. Remove `draft: true` once those
surfaces land, and re-check every command and endpoint on this page against what
actually shipped.
-->

# Secrets

A secret — an API key, an OAuth token, a mailbox password — must not sit in a node
property. Properties are returned by node reads, by `SELECT *`, by the audit log,
by `node:updated` events, and they replicate. RaisinDB therefore stores secrets in
a dedicated place and leaves a **reference** in the property.

```jsonc
// what you write
{ "api_key": "sk-live-abc123" }

// what is stored, and what every read returns
{ "api_key": "secret://node/01H8XY7Z9/api_key@1" }
```

The plaintext exists only inside the server, at the moment something uses it.

## Declaring a secret field

Mark the field `encrypted: true` in the NodeType, ElementType or Archetype:

```yaml
properties:
  - name: api_key
    type: String
    encrypted: true
    meta:
      label: API key        # presentation hints still live in meta
```

That is the whole opt-in. The server does the rest on write, so **no transport can
bypass it** — the same write over REST, WebSocket or SQL is vaulted identically.
Writing a value that is already a `secret://` reference passes through untouched,
so a round-trip (read a node, edit one field, write it back) does not re-vault or
mint a new version.

:::note
A nodetype created through SQL `CREATE NODETYPE` cannot declare a secret field
yet — there is no `ENCRYPTED` property modifier. Declare secret fields in YAML.
:::

:::tip Legacy spelling
Schemas that predate this feature used `meta: { secret: true }`. That is still
honoured, so shipped connector packages keep working. Prefer `encrypted: true` in
new schemas — it is a real schema field rather than a free-form hint.
:::

## Reading

**Reads never resolve a reference.** A `GET`, a `SELECT`, a WebSocket
subscription and an audit entry all return the `secret://…` string. This is the
point: a reference is safe everywhere a property goes, so there is no redaction
layer to forget to apply, and no read path that has to be kept in sync.

To *use* a secret you ask for it explicitly — from a function, or from the server
code that dials the remote service.

## References

```
secret://<name>            # newest version
secret://<name>@<version>  # pinned to one version
```

A name may contain `/`. For a vaulted schema field the server generates
`node/<node_id>/<field.path>`, which is stable across renames and moves — the node
id does not change when you move a node, so the reference never dangles.

Nested fields use the same dot path the rest of RaisinDB uses:
`node/<id>/hero.credentials.token`, `node/<id>/stops.0.key`.

## Managing secrets directly

Not every secret belongs to a node field. Operator-owned secrets — a shared API
key, a webhook signing secret — can be managed on their own.

```bash
raisindb secret set stripe-key      # reads the value from stdin, never argv
raisindb secret list                # names and metadata only
raisindb secret show stripe-key     # metadata; never the value
raisindb secret rotate stripe-key
raisindb secret rm stripe-key
```

Over HTTP, under `/api/secrets/{repo}/{branch}`, admin-gated. Note what is *not*
there: **no endpoint returns a plaintext value.** `list` and `show` return names,
versions, timestamps and the key id.

## Using a secret from a function

```js
const key = await raisin.secrets.get("stripe-key");
```

This is **deny-by-default**. A function may only read the secrets its own
`secret_policy` allowlist names, exactly like `network_policy` gates outbound
HTTP:

```yaml
secret_policy:
  allow: ["stripe-key", "ms-*"]
```

This matters more than it might look. Virtual-node adapters run privileged with
row-level security bypassed *and* hold `raisin.http` — so an unrestricted secrets
binding would be a one-line path to exfiltrating every credential in the
repository. Grant the narrowest pattern that works.

Functions with the right grant can also write:

```js
await raisin.secrets.put("ms-token", token);     // -> { name, version }
await raisin.secrets.rotate("stripe-key", next);
await raisin.secrets.list();                     // names + metadata
```

## Versions, rotation and history

Every write appends a version rather than replacing one. That is what makes
rotation safe: add the new value, let consumers pick it up, and anything still
holding `@2` keeps working until it is updated.

It is also what keeps **time travel** honest. An older node revision references
the version that was current when it was written, so reading history resolves to
what the node actually held at the time — not to today's value.

Deleting a node tombstones its secrets; older versions stay readable so historical
revisions still resolve. Clearing the field alone leaves earlier versions intact
for the same reason.

## Branches

Secrets are branch-scoped and are copied when you fork a branch, so a feature
branch can hold test credentials without touching production. Promoting content
between branches carries the referenced secrets with it.

## Keys and rotation

Secrets are sealed with AES-256-GCM under a master key that lives **outside the
database**, which is what makes a stolen RocksDB backup inert.

```bash
RAISIN_MASTER_KEYS="1:<64 hex chars>,2:<64 hex chars>"
RAISIN_MASTER_KEY_ACTIVE=2
```

Every blob records which key sealed it, so old and new keys coexist and rotation
is rolling rather than a flag day: deploy every node with both keys, then move
`RAISIN_MASTER_KEY_ACTIVE` forward one node at a time.

The older single `RAISIN_MASTER_KEY` still works and is treated as key id `0`.

:::warning Every node needs the same keys
Ciphertext replicates; keys do not. A node that receives a secret sealed under a
key it does not have will say so explicitly — `unknown key id 2 (have: [0, 1])` —
rather than failing obscurely. Keep the key set identical across the cluster.
:::

:::danger Development keys
With no key configured, a dev-mode server uses a publicly-known all-zero key.
Anything sealed with it is marked as such, and the server refuses to open those
blobs when it is not in dev mode — so a development database promoted to
production fails loudly instead of looking encrypted while being readable by
anyone.
:::

## What this does not cover yet

Secrets written before this feature — connector client secrets and OAuth token
blobs stored as `*_encrypted` node properties — have not been migrated. They are
encrypted, but they are still properties, so they still appear in `SELECT *` for
an administrator. Treat `raisin:system` as privileged until that migration lands.
