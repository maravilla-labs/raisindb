# Writing bytes to a provider: the drive write path

**Status:** design, not built. Written 2026-08-29 after the Microsoft 365 mount
bundle shipped read-only drive mounts and the obvious next question — "can I
create a file in RaisinDB and have it land in OneDrive?" — turned out to have a
firm answer: **not today**, and the reason is one missing channel rather than a
missing feature.

## The finding

The write drain sends the adapter exactly three shapes, and none of them can
carry bytes:

```rust
// write/create.rs:223      { payload, parent_id }
// write/push.rs:191        { item_id, payload, fields, etag }
// write/deletes.rs:178     { item_id, policy, etag }
```

`payload` is the mapper's `to_external` output, and `to_external` is pure and
I/O-free by contract. A `raisin:Asset`'s bytes live in the binary store behind
`node.properties.file.metadata.storage_key`; nothing in the drain reads them.
`grep -rn "put_content\|set_content\|upload_content"` returns zero hits.

So a `mirror` mount over a file-shaped provider syncs **metadata only** today.
The Microsoft 365 adapter states this honestly by declaring no write
capabilities for `files` at all, and refusing the resource in `opCreate` /
`opUpdate` — because declaring a capability without the implementation behind
it lets a mount resolve to a writable mode and then throw at drain time, after
the engine has already claimed the work.

Two pieces of collateral damage are worth recording, since both are evidence
that the gap was never deliberate:

- The adapter contract's op table documented `content` / `mime_type` / `name`
  params on `create` and `update` that the engine has never sent. Corrected in
  `docs/reference/virtual-node-adapters.md`.
- The Google Drive adapter carries a complete multipart-upload path branching
  on `params.content` (`google-drive/index.js:383, 402-419`) that **can never
  execute**. It was written against that table.

## What already exists

This is the reason the change is bounded rather than structural.

| Piece | State |
| --- | --- |
| `BinaryRetrievalCallback` (`storage_key -> Vec<u8>`) | exists, `package_install/types.rs:243` |
| …and is in scope where the sync handler is built | `init_system/mod.rs:64`, used at `:258` |
| …but is not forwarded to it | `integration_handlers.rs:41-53` takes only the write-side `binary_store` |
| Adapter fetch with a binary body | exists, `execution/callbacks/http.rs:193` decodes `bodyBase64` into real bytes |
| Engine-side transfer under operator egress policy | exists on the READ side, `content/fetch_url.rs` |
| Mirror plumbing (create/update/delete, adoption, etags) | exists and is exercised by the calendar surface |

The read path is the template for all of it. It accepts three answers from
`get_content` — `content_base64`, `content`, or a `fetch_url` the **engine**
downloads in Rust — precisely because a JS string is the wrong place for large
bytes. The write path should mirror that, in the same order of preference.

## The size problem, stated numerically

Base64 through the QuickJS boundary is not a general answer:

- read path caps one object at 64 MiB (`content/decode.rs:19`)
- the ms-graph adapter's whole memory budget is 64 MiB (`resource_limits.max_memory_bytes`)
- base64 is 1.33×, and the payload crosses `__raisin_call` via
  `JSON.stringify` / `JSON.parse` — two more full copies

**Realistic ceiling: ~10–15 MB.** Fine for a contract or a spreadsheet, useless
for video. Microsoft draws its own line at 4 MiB: a simple
`PUT …/items/{parent}:/{name}:/content` above that is a 413, and larger files
need `createUploadSession` + ranged `PUT`s to a **pre-authenticated** URL whose
final response carries the created driveItem.

## Design

Two phases, and phase 1 is genuinely useful on its own.

### Phase 1 — small files, bytes inline

1. **Engine:** forward `binary_retrieval` into the virtual-mount sync handler
   (`integration_handlers.rs` → `virtual_mount_sync/mod.rs` → `SyncCtx`).
2. **Engine:** when a node being created or updated carries a `file` Resource
   and the mount's adapter declares `accepts_content`, read the bytes and add
   `content_base64`, `mime_type` and `name` to the `create` / `update` params —
   the params the contract already documented.
3. **Engine:** cap it (proposed 8 MiB) and, above the cap, fail the ITEM with a
   reason naming the size. Not the run, and never silently: a skipped upload
   that reports success is the failure mode this whole area is prone to.
4. **Adapter:** forward `bodyBase64` in `graphFetch` (one line,
   `ms-graph/http.js:159`), implement the `files` branch of `opCreate` /
   `opUpdate` as a simple PUT, declare `can_create` / `can_update` /
   `can_delete` + `supports_trash` for `files`, and write the
   `/mappers/ms-graph-files` `to_external`.

### Phase 2 — large files, engine-streamed

The write-side mirror of `fetch_url`: the adapter answers `create` with
`{ upload: { url, method, headers, chunk_size } }` instead of an
`external_id`, the engine streams the bytes to that URL in Rust, and then calls
a new `finalize_upload` op with the provider's final response so the **adapter**
parses out `external_id` and `etag`. Provider-shaped parsing must not move into
the engine, which is the reason for the second call rather than having the
engine read the driveItem itself.

`createUploadSession` is an ordinary JSON POST and needs no new capability; the
ranged PUTs go to a host outside the adapter's `allowed_urls`, which is another
reason the transfer belongs in the engine rather than in the adapter's fetch.

## Contract details that will bite

- **A create that returns no `external_id` is a failure, and the node is not
  adopted** (`write/create.rs:243-257`). Deliberate: adopting with a fabricated
  id makes the node undeletable and unmatchable, and the next reconcile creates
  a second copy at the provider. An upload path must therefore not report
  success until it has the real id.
- **`create_node_types` is empty by default** and is the only ownership signal
  for a locally-born node — nothing is created remotely until an operator names
  the types. Engine-authored scaffolding folders are excluded separately
  (`create.rs:80-99`), because a mirror mount with `raisin:Folder` in its list
  once pushed its own scaffolding to the provider.
- **`missing_mirror_ops`** demands `can_delete` only when the mount actually
  propagates deletes; a `detach` mount never calls the adapter. `purge` is never
  a default at any layer — an operator has to type it.

## Credential work (parallel, and mostly not code)

Write scopes are their own project: `Files.ReadWrite`, `Files.ReadWrite.All`,
`Sites.ReadWrite.All` must be added to both connector templates, to Connect's
registry ceiling in **two** files (`config/providers.example.toml` and
`cloud-ops/roles/connect/templates/providers.toml.j2` — a test pins the first
against the templates), and to each existing tenant's client allowlist by direct
Mongo `$set`, because there is no admin endpoint for it and **re-minting rotates
the client secret**, which breaks the already-provisioned Integration node.
Then a Microsoft admin adds the delegated permissions to Maravilla's app, each
customer's admin re-consents, and every connected account reconnects — a token
refresh never widens consent, and a package re-install never reaches a live
connector node.

A widen-only `PATCH /admin/clients/{client_id}` in Connect would remove the
Mongo step and is small.

## Recommendation

Phase 1 is a day's work spread across two repos and is enough for documents,
which is what people actually ask for. Phase 2 is the honest answer for media
and should not be skipped, but it can follow. Do the scope work first and
independently: it has human steps with real latency (two admin consents and a
reconnect per account), and nothing can be tested end to end until it lands.
