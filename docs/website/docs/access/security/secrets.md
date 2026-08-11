---
sidebar_position: 4
---

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
echo -n "sk-live-abc123" | raisindb secret set stripe-key   # value from stdin, never argv
raisindb secret list                # names and metadata only
raisindb secret show stripe-key     # metadata + every version; never the value
raisindb secret rotate stripe-key   # value from stdin, as with set
raisindb secret rm stripe-key --yes # appends a tombstone
```

`set` and `rotate` read the value from **stdin by default** — no flag — because
the spelling that feels natural (`secret set NAME value`) is exactly the one that
writes the credential into shell history and exposes it in the process table. In
CI, `--value-env VAR` reads it from the environment instead.

Both print the reference to paste into a node property, so you never assemble it
by hand:

```
Secret 'stripe-key' written to shop/main as version 1 (value from stdin).
  Reference (paste into an encrypted property): secret://stripe-key
```

Every command is branch-scoped: `--repo` and `--branch` (default `main`), with
`--repo` falling back to the configured default repository. There is no
`secret get` — no API returns a plaintext value, and a command that printed one
would put credentials into terminal scrollback and CI logs.

Over HTTP, under `/api/secrets/{repo}/{branch}`, admin-gated:

| Method | Path | Returns |
|---|---|---|
| `PUT` | `/api/secrets/{repo}/{branch}/{name}` | `{ name, version, reference }` |
| `GET` | `/api/secrets/{repo}/{branch}` | `{ secrets: [...] }` — newest version of each name |
| `GET` | `/api/secrets/{repo}/{branch}/{name}` | the newest version's metadata, plus `versions: [...]` |
| `DELETE` | `/api/secrets/{repo}/{branch}/{name}` | `{ name, version, deleted: true }` |
| `POST` | `/api/secrets/{repo}/{branch}/rotate/{name}` | `{ name, version, rotated, reference }` |

Note what is *not* there: **no endpoint returns a plaintext value.** `list` and
`show` return names, versions, timestamps and the key id.

A name may contain `/` (the vaulted-field convention is
`node/{id}/{field.path}`), so `{name}` is a wildcard capture that runs to the end
of the path. That is why rotation reads `rotate/{name}` and not `{name}/rotate` —
a wildcard has to be the last segment.

Send those slashes **literally**, not as `%2F`:

```
PUT /api/secrets/shop/main/node/01H8XY/api_key
```

An encoded `%2F` is decoded during capture and reaches the same secret, so it is
a redundant spelling rather than a distinct name — but intermediaries normalise
it inconsistently, so the literal form is the one to send.

The same operations are on the WebSocket API as `secret_put`, `secret_rotate`,
`secret_list`, `secret_get` and `secret_delete`, with `{repo, branch}` taken from
the connection's request context. `secret_get` returns **metadata**; as over
HTTP, there is no request that returns a value.

## Using a secret from a function

The common case is a secret stored in a node field. The property holds a
reference, and reads never resolve it, so the function is handed the reference
string — pass it straight through:

```js
const node = await raisin.nodes.get(id);
node.api_key                                  // "secret://node/01H8XY.../api_key@1"
const key = await raisin.secrets.get(node.api_key);
```

`get()` takes either a full reference or a bare name, so an operator-managed
secret works the same way:

```js
const key = await raisin.secrets.get("stripe-key");
```

**Do not strip the `secret://` prefix or the `@version` suffix yourself.** A
secret name may itself contain `@`, and only a trailing run of digits after the
*last* `@` is a version — get that wrong and you silently read someone else's
secret, or the wrong version of your own.

A version pinned in the reference is honoured: `secret://k@1` reads version 1,
not the newest. That is what makes reading an older node revision give the value
that revision actually held. `get()` also takes an explicit version argument —
`get("stripe-key", 2)`, or `get("secret://stripe-key", 2)` — but passing one
*alongside a pinned reference* throws, because two stated versions cannot both
be satisfied and quietly preferring either would defeat the guarantee the pin
exists for.

For a config value that *might* be a reference, `resolve()` handles both:

```js
const host = await raisin.secrets.resolve(cfg.host);      // plain value, unchanged
const pass = await raisin.secrets.resolve(cfg.password);  // reference -> plaintext
```

A plain literal passes through untouched and needs no grant. A reference that
fails to resolve **throws** rather than returning the reference string — sending
the literal `secret://…` to a provider as an API key would only surface as a
baffling 401 far from the cause.

This is **deny-by-default**. A function may only read the secrets its own
`secret_policy` allowlist names, exactly like `network_policy` gates outbound
HTTP. Declare it in the function's `.node.yaml`:

```yaml
secret_policy:
  enabled: true
  allowed_names:
    - "stripe-key"
    - "ms-*"
```

Both fields are required in substance. Omitting `enabled: true` denies
everything, and so does an empty `allowed_names` — "opted in" never silently
means "unrestricted". A denial names the secret and which of those two rules
applied, so a missing grant reads as a missing grant rather than a missing
secret.

This matters more than it might look. Virtual-node adapters run privileged with
row-level security bypassed *and* hold `raisin.http` — so an unrestricted secrets
binding would be a one-line path to exfiltrating every credential in the
repository. Grant the narrowest pattern that works.

The allowlist is matched against the **parsed name**, so the same secret gets the
same answer whichever spelling you pass — a grant of `node/*` covers
`secret://node/01H8.../api_key@1` without also listing the reference form.

Functions with the right grant can also write:

```js
await raisin.secrets.put("ms-token", token);     // -> { name, version }
await raisin.secrets.rotate("stripe-key", next);
await raisin.secrets.list();                     // names + metadata
await raisin.secrets.delete("old-key");
```

Writes always append a version, so a write against a **pinned** reference
(`secret://k@1`) is refused rather than silently ignoring the pin — otherwise it
would look as though version 1 had been replaced.

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

## Branches, copies and promotion

Secrets are branch-scoped and are copied when you fork a branch, so a feature
branch can hold test credentials without touching production. Promoting content
between branches carries the referenced secrets with it, so a promoted node can
still resolve on the target.

**Copying a node gives the copy its own secret.** A copy gets a new node id, and
the secret name is derived from it, so the copy is re-sealed under its own name
rather than sharing the original's. That is what stops deleting the source from
taking the copy's password with it — and it means changing one afterwards does
not change the other, which is usually what you want but is worth knowing if you
expected them to stay in step.

Promotion is the opposite case and behaves the opposite way: it preserves the
node id, so both branches genuinely reference the same secret.

## Keys and rotation

Secrets are sealed with AES-256-GCM under a master key that lives **outside the
database**, which is what makes a stolen RocksDB backup inert.

```bash
RAISIN_MASTER_KEYS="1:<64 hex chars>,2:<64 hex chars>"
RAISIN_MASTER_KEY_ACTIVE=2
```

Old and new keys coexist, so rotation is rolling rather than a flag day: deploy
every node with both keys, then move `RAISIN_MASTER_KEY_ACTIVE` forward one node
at a time. **Keep retired keys in `RAISIN_MASTER_KEYS`** — anything still sealed
under one stops opening the moment you drop it.

The older single `RAISIN_MASTER_KEY` still works and is treated as key id `0`.

:::warning Every node needs the same keys
Ciphertext replicates; keys do not. Keep the key set identical across the
cluster, or a node will receive secrets it cannot open.
:::

### Envelope versions, and what is not active yet

There are two on-disk envelope formats. The reader accepts both, always. Which
one gets *written* is gated:

| | v1 (written today) | v2 (`RAISIN_CRYPTO_EMIT_V2=1`) |
|---|---|---|
| key id in the blob | no | yes |
| bound to tenant/repo/field (AAD) | no | yes |
| opening a blob | trial-decrypt across the keyring | direct lookup by key id |

**v1 is still the default**, because a v2 blob cannot be read by a node running
an older binary — the format cannot flip in the same release that learns to read
it. Until you set `RAISIN_CRYPTO_EMIT_V2`, three things do not apply:

- **Blobs do not record which key sealed them.** Rotation still works, because a
  v1 blob is opened by trying each key in the ring, but a blob no key opens
  reports a plain decryption failure rather than naming the missing key id.
- **AAD binding is not in effect.** A v2 blob is bound to its tenant, repo and
  field; a v1 blob is not, so it would still decrypt if moved elsewhere.
- **The development key is not marked** (see below).

Turn the gate on only once every node in the cluster is running a binary that can
read v2 — that is the whole reason it is a separate step.

:::danger Development keys
With no key configured, a dev-mode server uses a publicly-known all-zero key —
anything sealed with it is readable by anyone.

Once `RAISIN_CRYPTO_EMIT_V2` is on, such blobs are tagged with a reserved key id
and the server refuses to open them outside dev mode, so a promoted development
database fails loudly instead of merely looking encrypted. **Under the current v1
default there is no such tag and no such refusal** — a dev database promoted to
production will keep working, silently, under a key that is public knowledge.
Never promote one; re-seal the secrets instead.
:::

## What this does not cover yet

Secrets written before this feature — connector client secrets and OAuth token
blobs stored as `*_encrypted` node properties — have not been migrated. They are
encrypted, but they are still properties, so they still appear in `SELECT *` for
an administrator. Treat `raisin:system` as privileged until that migration lands.
