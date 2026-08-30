# Core versus plugin, and what has to be true on the box

**Scope.** What RaisinDB core does with a binary asset on its own, what only the
Maravilla media plugin can do, how a pipeline is supposed to choose between
them, and what must be true on a Hetzner box for `raisin.media.*` to work at
all. Every claim about core's column is anchored to a file and a line read in
this worktree, not to a feature-flag name.

**The owner's framing, which this document takes as the constraint:** core must
be rock solid on its own foundations. The plugin may do much more — Word,
PowerPoint, other transformations — which raisindb natively cannot, and that is
fine. But the plugin exists only on the Studio installation, it must work
*there*, and there is a further API of the owner's (Delivery) that does not
exist in this repo and is a hard prerequisite.

Status of the code shipped alongside this document is in
[§7 Built vs designed](#7-what-i-built-and-what-i-only-designed).

---

## 1. The capability table

### 1.1 What "core does it" means, and where that is decided

Core's entire binary-asset text vocabulary is **one function with one arm**:

```rust
// crates/raisin-rocksdb/src/jobs/handlers/asset_processing/helpers.rs:224
pub(crate) fn is_extractable_mime(mime_type: &Option<String>) -> bool {
    matches!(mime_type.as_deref(), Some("application/pdf"))
}
```

and its dispatch partner `process_extractable` (`helpers.rs:233`) returns `None`
for everything else, so the caller can say so rather than silently succeed.
Images are separated by `is_image_mime` (`helpers.rs:245`, a `image/` prefix
test) and get a CLIP embedding when `options.generate_image_embedding` is set
(`asset_processing/handler.rs:358`). Captioning is compiled in but explicitly
disabled — `log_deprecated_captioning_warning` (`handler.rs:364`, `:572`).

For the function surface, PDF is unconditional in any build that has
`raisin-functions` at all:

```toml
# crates/raisin-functions/Cargo.toml:26
raisin-ai = { path = "../raisin-ai", features = ["pdf", "pdf-markdown", "ocr"] }
```

surfaced to guest code as `raisin.pdf.extractText / getPageCount / ocr /
processFromStorage` (`runtime/quickjs/api_wrapper.js:1037-1075`) and as
`resource.processDocument()` (`api_wrapper.js:176-190`), with
`pdf_process_from_storage` on the API trait (`api/traits.rs:233`).

Everything else is absent, and absent by evidence rather than by omission:
grepping the workspace for `docx|pptx|xlsx|officedocument|odt|libreoffice`
returns only virtual-mount **test fixtures** — there is no office-format code in
any crate. Same for ffmpeg, video and audio.

### 1.2 The table

| mimetype | RaisinDB core, today | Needs the plugin | Nobody does it |
|---|---|---|---|
| `.pdf` `application/pdf` | **text + OCR.** `raisin-ai::pdf` (pdf_oxide markdown, pdf-extract native, tesseract OCR) via `raisin.pdf.*`; and the asset job, whose whole vocabulary is this one mimetype (`is_extractable_mime`). Text now lands on the node — `persist_extracted_text` (`asset_processing/handler.rs:407-480`) writes the `extracted_text` property, so it reaches embedding and fulltext. | thumbnail (`media.doc.thumbnail`), → docx/html (`media.doc.convert`), QR insertion, image replacement, template merge | — |
| `.png .jpg .tiff .webp` | **CLIP embedding + tesseract OCR** (`is_image_mime` + `generate_image_embedding`, `handler.rs:358`). Captioning compiled in and **deliberately disabled**. | `media.image.resize`, `media.image.detectFaces`, and the ImageMagick fallback path | — |
| `.docx` `.pptx` `.xlsx` `.odt` | **nothing.** No code in any crate. | everything: `media.doc.toPdf / convert / toMarkdown / toHtml / thumbnail / replaceImages / insertQrCode / templateMerge` — LibreOffice + python3-uno + pandoc + ghostscript, all on the Delivery side | — |
| `.mp4 .mov .webm` | **nothing.** | `media.video.transcode / thumbnail / extractFrames / extractAudio / render` — ffmpeg + `chrome-headless-shell` | — |
| `.mp3 .wav .m4a` | **nothing.** | `media.video.extractAudio` gets the bytes out of a container | **transcription.** No `whisper`, no STT anywhere, in core or plugin. Audio is opaque to search. |
| `.txt .md .csv` | **nothing as an uploaded asset.** The same text in a node *property* is embedded normally (`embedding/content_extraction.rs`); as binary bytes it is never opened. | — (the plugin has no text-file op either) | **plain-text asset extraction.** The cheapest possible extractor is the one nobody wrote. |
| `.html` (stored file) | **nothing.** | `media.browser.extractHtml` (hydrated DOM, headless Chrome) | — |
| an HTML **page** → PNG/PDF | **nothing.** | `media.browser.screenshot`, `media.browser.pdf` | — |

The plugin's method list is not guessed; it is `KEYED_OPS` + `REQUEST_OPS` +
`UTILITY_OPS` in `maravilla-runtime/crates/maravilla-media-plugin/src/dispatch.rs:18-50`,
flattened by `method_names()` into exactly the strings declared to the host.

**Read the last column.** Two gaps are real product gaps rather than a
plugin-deployment question: **audio has no transcription anywhere**, and **plain
`.txt`/`.md`/`.csv` uploads are never opened** even though extracting them needs
no dependency at all. Both are silent today: the bytes are stored, the asset
appears in Studio, and search returns nothing.

### 1.3 Why the split is right

Chrome, ffmpeg, ImageMagick 7, LibreOffice + UNO, pandoc and ghostscript live on
the **Delivery** side (cloud-ops `roles/media-transforms`, `roles/libreoffice`)
and never enter the raisindb process. That is the whole value of the boundary: a
LibreOffice crash, a Chrome memory spike or a font package cannot take the
database down, and raisindb's release cadence is not chained to theirs. Nothing
in this document argues for pulling any of it into core.

---

## 2. The degradation contract

### 2.1 What it was before this change

There was no capability probe **anywhere**. `registered_function_plugins()`
(`plugin.rs:79`) was referenced only inside `plugin.rs` itself — no HTTP
handler, no SQL function, no JS binding. The estate's only probe was hand-written
inside one Studio function:

```js
// studio/…/capture-page-screenshot/index.js:181
if (!(raisin.media && typeof raisin.media.screenshot === 'function'))
  return { ok: false, reason: 'not_configured' };
```

and the media plugin's own companion function did not do it —
`job-poll/index.js:118` calls `raisin.media.job(...)` unguarded. Two functions
in one estate, depending on one plugin, with opposite failure behaviour. That is
precisely the mirrored-path drift CLAUDE.md names as this repo's #1 recurring
bug class, sitting on the plugin boundary.

And the unguarded one was the dangerous one, because **an absent plugin threw**.
`assemble_plugin_js()` returned an empty `String` when no plugin was registered
(`plugin.rs:97-101` before this change), so `globalThis.raisin.media` was
`undefined` and the first property access was a `TypeError` — while
`maravilla-media-plugin/package/manifest.yaml`, the plugin README and
`job-poll/index.js:118-127` all describe the absent case as
`{ ok:false, reason:'unknown_method' }` data. A TypeError aborts the whole
invocation, so the poller never reached its own failure handling: every
`maravilla:MediaJob` froze at `pending`, spinner spinning, nothing recorded.

### 2.2 The three failure layers, named precisely

They are genuinely different states and a pipeline must be able to tell them
apart:

| reason | means | fix lives |
|---|---|---|
| `plugin_absent` *(new)* | no plugin provides this namespace on this server | deploy: the `.so` is missing, or was rejected (ABI) |
| `not_configured` | plugin loaded; `DELIVERY_MEDIA_URL` or `INTERNAL_API_TOKEN` empty (`media-plugin/src/delivery.rs:53-64`) | ops: server env |
| `delivery_unreachable` | plugin loaded and configured; Delivery down, wrong port, or slower than the 30 s ceiling (`delivery.rs:22`) | ops: the Delivery service |
| `unknown_method` | plugin loaded and configured; the method name is a typo (`dispatch.rs:72-86`) | the calling function |
| thrown `Error` | host-side failure: method not in the map, bad args JSON, join error → `{error:true}` → `__pcall` throws (`plugin.rs`, `gateway.rs:108-118`) | a bug, not a config |

### 2.3 The contract, as a pipeline writes it

```js
// The probe. Present in EVERY build, plugin or no plugin.
if (raisin.capabilities.has('media.doc.toPdf')) {
  const job = raisin.media.doc.toPdf(storageKey, opts);   // plugin path
  ...
} else if (mime === 'application/pdf') {
  const out = await raisin.pdf.processFromStorage(storageKey, { ocr: true });
  ...                                                      // core fallback
} else {
  await recordUnsupported(node, mime, 'no plugin and no core extractor');
}
```

and, because guest code will still be written by people who do not read this
document, calling an absent binding is now *also* safe:

```js
raisin.media.doc.toPdf('k', {})
// → { ok: false, reason: 'plugin_absent', method: 'media.doc.toPdf' }
```

**Where the probe lives.** One place: `crate::plugin` in `raisin-functions`,
which owns the registry. `raisin.capabilities` is generated inside
`assemble_plugin_js()` from `registered_plugin_methods()` — the same registry the
dispatcher reads, so the probe cannot disagree with what a call will do. There is
no second table.

**Caching and invalidation.** The plugin directory is scanned exactly once, at
startup, before any function executes (`load_plugins_from_dir`, called from
`main.rs:79-95`), and registration is append-only into a process-global. So the
capability set **cannot change without a restart**, and the probe is baked into
the `LazyLock`-cached wrapper source (`runtime/quickjs/environment.rs:196-204`)
that is built once per process. Invalidation is process restart, and nothing
else. That is a feature: no TTL, no refresh path, no window where two isolates
disagree.

### 2.4 The hard rule, and the trace it requires

> **A missing plugin must never silently drop a document from the index.**
> Every unsupported or unhandled asset must leave a durable trace that a later
> backfill can find by query.

A `return` with no record is indistinguishable from "there was nothing to do" —
the exact shape of the two silent gaps in §1.2. The trace design (see §7: this
part is designed, not built):

Write onto the asset node, in one place, a property bag:

```json
"media_processing": {
  "state": "unsupported",
  "mime_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  "reason": "plugin_absent",
  "capability": "media.doc.toPdf",
  "server_capabilities_version": "<hash of the capability report>",
  "recorded_at": "2026-08-30T09:12:00Z",
  "attempts": 1
}
```

Five properties of this shape matter, and each answers a specific failure:

1. **`state` is an enum, not a boolean** — `done | unsupported | failed |
   pending`. A boolean cannot distinguish "we looked and cannot" from "we have
   not looked yet", and the backfill query needs that difference.
2. **`reason` is the §2.2 vocabulary verbatim.** A backfill after a plugin
   rollout selects `reason = 'plugin_absent'`; a backfill after a Delivery fix
   selects `delivery_unreachable`. Collapsing them into one "failed" makes both
   backfills re-do the other's work.
3. **`capability` names the method that was missing**, so the backfill can be
   *conditional on the probe*: re-run exactly the nodes whose missing capability
   is now present.
4. **`server_capabilities_version`** is a hash over the capability report. It is
   what makes the trace self-invalidating: a node marked `unsupported` under a
   capability set that no longer applies is a candidate, without anyone
   remembering which deploy changed what.
5. **It is a node property, not only a job record.** Job records are pruned
   (`JobRegistry` sweeps; dedup is per-process anyway — see CLAUDE.md), and the
   backfill must be a plain SQL query months later:

```sql
SELECT path, properties->>'media_processing'::String
FROM 'assets'
WHERE properties->>'media_processing.state'::String = 'unsupported'
  AND properties->>'media_processing.reason'::String = 'plugin_absent';
```

**Where it must be written.** Not in `NodeService` — CLAUDE.md is explicit that
it is not a write chokepoint (SQL DML and the WS create handler bypass it). The
asset pipeline's own handler is the right place: `persist_extracted_text`'s
sibling in `asset_processing/handler.rs`, which already has the node, the
options and a write path that emits `node:updated`. One writer, one format.

---

## 3. What must be true on the Hetzner box

Six things. Each is currently provisioned; each has a failure mode that is
silent today.

**1. A linux-gnu cdylib exists for the exact plugin version.**
Built by `maravilla-runtime/.github/workflows/media-plugin-release.yml` on a
`media-plugin-vX.Y.Z` tag, producing
`libmaravilla_media_plugin-vX.Y.Z-x86_64-unknown-linux-gnu.so` and publishing it
into `maravilla-labs/maravilla-cli` at tag `vX.Y.Z`.

**2. It is at `<data_dir>/plugins/`, owned by the raisindb user, mode 0755.**
`cloud-ops/roles/raisindb/tasks/main.yml:106-169` creates the directory, resolves
the asset URL, downloads, uploads and chmods — every task guarded by
`when: maravilla_media_plugin_version | length > 0`, and the upload
`notify: restart raisindb`.

**3. The server is configured to look there and is a rocksdb build.**
`RAISIN_PLUGIN_DIR={{ raisindb_data_dir }}/plugins`
(`roles/raisindb/templates/raisindb.env.j2:81`), read at `main.rs:81-82`. Note
the whole plugin block is inside `#[cfg(feature = "storage-rocksdb")]` — a
non-rocksdb build loads **no plugins and logs nothing at all**.

**4. The plugin's ABI matches the host's.**
`RAISIN_PLUGIN_ABI_VERSION = 1` (`plugin_loader.rs:49`); `load_one` compares it
first and returns `Err("ABI version mismatch (plugin=…, host=…)")`
(`plugin_loader.rs:190-196`), which `load_plugins_from_dir` logs as
`skipping plugin` and continues past. **The server boots green with every media
binding gone.** Nothing in cloud-ops asserts compatibility:
`raisindb_version: "0.3.59"` and `maravilla_media_plugin_version: "0.2.1"`
(`group_vars/all/vars.yml:337,344`) are fully independent variables with no
matrix and no guard.

**5. Delivery — the owner's separate API — is running and reachable.**
This is the prerequisite that does not exist in this repo. It is built and
released from **`maravilla-labs/flightdeck`**, pinned by `flightdeck_version`
(`cloud-ops/roles/delivery/tasks/main.yml`), and its surface is
`flightdeck/delivery/src/handlers/media_internal/mod.rs`. For `raisin.media.*`
it must expose, on `http://127.0.0.1:3001`, behind
`verify_internal_token_header` (`x-internal-token`):

| route | serves |
|---|---|
| `POST /_internal/media/transforms` | enqueue a source-keyed transform (all 15 `KEYED_OPS`) |
| `GET /_internal/media/transforms/jobs/{id}` | job status — what `media.job` polls |
| `POST /_internal/media/browser/screenshot` | headless Chrome → PNG |
| `POST /_internal/media/browser/pdf` | headless Chrome → PDF |
| `POST /_internal/media/browser/extract-html` | hydrated DOM |
| `POST /_internal/media/frames/render` | `media.video.render` |
| `GET`/`PUT /_internal/storage/{*key}` | fetch an output / stage an input |

and the transforms worker behind it needs ffmpeg, tesseract + language packs,
ImageMagick 7 (`roles/media-transforms`), LibreOffice + python3-uno + pandoc +
ghostscript + CJK/emoji fonts (`roles/libreoffice`), and `chrome-headless-shell`
at `/opt/chrome-headless-shell-linux64/` — full Chrome answers `-32601` to
`HeadlessExperimental.beginFrame` and will not do.

**6. One shared credential, matched on both sides.**
`DELIVERY_MEDIA_URL` and `INTERNAL_API_TOKEN` in raisindb's env
(`raisindb.env.j2:86-88`) must match Delivery's `INTERNAL_API_TOKEN`
(`delivery.env.j2:22`). Both currently resolve to `vault_muli_api_key`. The
plugin reads them straight from `std::env` and returns `None` — hence
`not_configured` — if either is empty (`media-plugin/src/delivery.rs:53-64`).
Caddy's `@internal path /_internal/*` 404 rule keeps the surface off the edge.

### 3.1 It must fail at startup, not at first upload

`delivery_unreachable` is a *documented* failure mode, which means today a
misconfiguration is discovered by a customer at first upload, weeks after the
deploy that caused it. That is backwards. The startup contract should be:

- **Plugin present but its config env is empty** → **WARN at boot**, naming
  `DELIVERY_MEDIA_URL` / `INTERNAL_API_TOKEN`. Not fatal: a dev machine
  legitimately runs the plugin without Delivery.
- **Plugin file present in the directory but rejected** → **ERROR at boot**,
  naming both ABI numbers, *and* recorded (built — §7) so a deploy can assert on
  it rather than grepping the journal.
- **`RAISIN_PLUGIN_DIR` set explicitly to a directory that does not exist** →
  **ERROR**. Today a missing directory is a `tracing::debug` no-op
  (`plugin_loader.rs:160-164`), which is right for the default
  `<data_dir>/plugins` and wrong for a path an operator typed on purpose.
- **A configured, loaded plugin whose Delivery does not answer a cheap probe** →
  **WARN at boot** with the URL. This needs a `GET /_internal/health` on
  Delivery and a `media.health` method on the plugin; both are the owner's
  repos, not this one. It is the single highest-value thing to add to Delivery
  for this contract.

---

## 4. The deploy chain, tag → working install

The chain exists end to end. What follows marks every step where it can silently
ship something stale — this project has been bitten by exactly that before (see
the npm nested-copy trap and the cargo mtime trap in project memory).

| # | step | where | silent-staleness risk |
|---|---|---|---|
| 1 | Tag `media-plugin-vX.Y.Z` in `maravilla-runtime` | GitHub Actions `media-plugin-release.yml` | Fires only on a **lightweight** tag in some of this estate's workflows — an annotated tag is a silent no-trigger. Verify the run started; do not assume. |
| 2 | Build cdylibs (linux-gnu, darwin ×2, msvc), pack the `.rap`, emit `SHA256SUMS` | same workflow | — |
| 3 | Publish assets to `maravilla-labs/maravilla-cli` tag `vX.Y.Z` | `gh release upload … --clobber` | **⚠ Highest risk.** `--clobber` on an existing tag replaces the asset. `maravilla_media_plugin_version: "0.2.1"` therefore does not identify specific bytes — a workflow re-run silently changes what the next deploy loads *as native code into the raisindb process*. The stripe plugin already uses a namespaced `stripe-plugin-v*` tag; the media plugin does not. |
| 4 | Bump `maravilla_media_plugin_version` in `cloud-ops/group_vars/all/vars.yml:344` | git | — |
| 5 | `make raisindb` → `playbooks/deploy-raisindb.yml` | ansible | — |
| 6 | Resolve + download + upload the `.so`, chmod, `notify: restart raisindb` | `roles/raisindb/tasks/main.yml:115-169` | **⚠ `get_url` with no `checksum:`.** `SHA256SUMS` is published and never fetched. Compounds #3. |
| 7 | Restart handler runs; raisindb re-scans the plugin dir | systemd | **⚠ Nothing verifies the plugin loaded.** The play's only check is `GET /management/admin/jobs` → 200. An ABI-rejected plugin passes that check perfectly. |
| 8 | Studio SPA + `studio.rap` bundle deployed | `roles/studio`, `studio_rap_url` (`vars.yml:57-58`); flightdeck auto-installs the `.rap` per tenant | **⚠ `flightdeck_version: "latest"`** (`vars.yml:10`) — a floating pin on the service that installs it. |
| 9 | Install the companion `maravilla-media` package **per repo** | `make raisindb-media-package TENANT=x REPO=y` → `playbooks/install-media-package.yml` | **⚠ Not part of `make raisindb`.** `roles/raisindb/tasks/media_package.yml` is not `include_tasks`'d from `main.yml` at all. It is remembered, not declared. |

### 4.1 The asymmetry in step 9 that produces the worst symptom

The `.so` is **process-wide**: the moment it loads, `raisin.media.*` works for
every tenant and every repo. The companion package — the `maravilla:MediaJob`
nodetype and the `job-poll` function that drives the progress bar — is
**per-repo**, installed by hand, with no inventory of which repos have it or at
what version (`maravilla_media_package_version` is tracked separately at
`vars.yml:355`).

So a new Studio tenant gets **working submits and no progress tracking**, and it
presents as "the progress bar is broken" rather than "the package was never
installed here". Combined with the old TypeError (§2.1) it presented as nothing
at all.

### 4.2 The four fixes, in order of payoff

1. **Verify the load after the restart** (§5 gives the mechanism): an ansible
   task asserting the expected plugin name is present. This is the one that
   converts the ABI cliff from a silent estate-wide outage into a red deploy.
2. **`checksum:` on the `get_url`**, from the published `SHA256SUMS`; and refuse
   `--clobber` on an existing tag in the workflow, or move the media plugin to a
   `media-plugin-v*` tag namespace like the stripe plugin.
3. **Declare the media-package targets**: `media_package_targets: [{tenant, repo}]`
   in group_vars, looped from a playbook `make raisindb` runs, so the set is
   declared rather than remembered. Have the install read the version back and
   fail on mismatch.
4. **Pin plugin and package together** with a comment saying they install as a
   pair, plus `maravilla_media_plugin_min_raisindb` / `_max` guards asserted
   before the upload.

---

## 5. The startup health check

**Built** (§7). On boot, right after the plugin scan, the server now prints what
it can actually do:

```
INFO scanning for function plugins plugin_dir=/data/maravilla/raisindb/plugins
INFO function plugin loaded plugin=maravilla-media methods=23
INFO media capability pdf: plugin OK (maravilla-media)
INFO media capability docx: plugin OK (maravilla-media)
INFO media capability pptx: plugin OK (maravilla-media)
INFO media capability image: plugin OK (maravilla-media)
INFO media capability video: plugin OK (maravilla-media)
WARN media capability text: UNSUPPORTED
```

and on a core-only box:

```
INFO media capabilities: no function plugins loaded (core-only build); \
     raisin.media.* returns { ok:false, reason:"plugin_absent" }
INFO media capability pdf: core (text extraction (native + OCR fallback))
INFO media capability image: core (CLIP embedding + OCR)
WARN media capability docx: UNSUPPORTED
WARN media capability video: UNSUPPORTED
```

and on the box the deploy chain is most likely to produce by accident:

```
INFO media capability docx: UNSUPPORTED
WARN plugin REJECTED — its bindings are absent for every tenant on this server
     path=/data/maravilla/raisindb/plugins/libmaravilla_media_plugin.so
     reason=ABI version mismatch (plugin=1, host=2)
```

**Design notes.** The report is derived, not declared: `capability_report()`
resolves every row of one `MEDIA_KINDS` table against the live plugin registry,
so a plugin that stops declaring `media.doc.toPdf` demotes `docx` to
`UNSUPPORTED` without anyone editing a list. `UNSUPPORTED` is logged at WARN and
everything else at INFO, so a log-level alert catches a gap. Rejections are
*recorded* in the registry, not merely logged — that is what makes them
queryable by the endpoint below.

### 5.1 `GET /api/management/plugins` — designed, not built

The natural consumer of the same data, for the ansible assertion in §4.2:

```json
{
  "abi_version": 1,
  "plugin_dir": "/data/maravilla/raisindb/plugins",
  "loaded":   [{ "name": "maravilla-media", "methods": ["media.doc.toPdf", "..."] }],
  "rejected": [{ "path": "…/libmaravilla_media_plugin.so",
                 "reason": "ABI version mismatch (plugin=1, host=2)" }],
  "capabilities": [{ "kind": "docx", "provider": "plugin",
                     "plugin": "maravilla-media", "method": "media.doc.toPdf" }]
}
```

Everything it needs is already exported: `plugin_manifest()`,
`rejected_plugins()`, `capability_report()` — all `Serialize`, all in
`raisin-functions`, which `raisin-transport-http` already depends on
unconditionally (`crates/raisin-transport-http/Cargo.toml:63`). The handler is
~30 lines in `handlers/management/global.rs` plus one route in
`routes/management.rs`. It is not built here only because compiling
`raisin-transport-http` needed more disk headroom than this machine had
(4.8 GB free at the end of the session). **It should not report
configured-ness by echoing tokens** — a boolean "configured" per plugin, from a
future ABI addition that lets a plugin declare its config keys, is the right
shape.

---

## 6. How chunking config flows through all of this

**The plain answer to the owner's direct question: the admin console's chunking
panel is decorative. Setting chunk size, overlap or splitter there changes
nothing about how documents are chunked.**

Verified this session, not inferred:

1. The console writes chunking into `TenantAIConfig.embedding_settings`
   (`packages/admin-console/src/pages/TenantAiSettings.tsx` → `api/ai.ts:141,184`
   → `handlers/ai/config.rs:198-201`).
2. The live embedding job reads a **different record**:
   `TenantEmbeddingConfig.chunking`
   (`raisin-rocksdb/src/jobs/handlers/embedding/handler.rs:199-215`), written
   only by `POST /api/tenants/{t}/embeddings/config`.
3. `grep -rn embedding_settings crates/ --include=*.rs` outside `handlers/ai`
   returns **only** `raisin-server/src/embedding_worker.rs:295` and
   `embedding_worker/job_handlers.rs:40` — and neither `embedding_worker` nor
   `embedding_worker/` appears in `main.rs`'s module list (`main.rs:15-35`).
   **They are never compiled.** A dead second pipeline is what made the console
   look wired.

So: the operator sets 512/20 %/Markdown, sees "Configuration Saved", reloads and
sees it persisted — and the job keeps using `TenantEmbeddingConfig.chunking`,
whose default is `None`, i.e. no chunking at all.

Two further console surfaces have the same shape: the per-repository AI settings
page's Save button is a 500 ms `setTimeout` and a success toast
(`repository-settings/AISettings.tsx:57-71`), and processing-rule chunking
overrides round-trip to storage and are then dropped by the asset pipeline,
which copies twelve fields out of the matched rule and not `chunking`.

**Where chunking touches this document's subject.** Chunking is where core's own
foundations meet the plugin question, because a plugin-extracted `.docx` and a
core-extracted `.pdf` both end up as *text on a node* and go through the same
splitter. If the splitter is misconfigured, the plugin's extra reach buys
nothing:

- `chunk_size` is measured in **characters**, not tokens, unless a tiktoken
  model name is supplied (`raisin-ai/src/chunking/mod.rs:94-97,192-202`), so the
  console's "Chunk Size (tokens)" label is wrong by roughly 4× for English.
- `SplitterType` has **zero readers** in the whole repo — Markdown, Code and
  FixedSize all silently get the recursive splitter.
- `chunk_text` **panics on non-ASCII** (`mod.rs:106-112`: `current_offset =
  start_offset + 1` lands mid-character), which is a panic, not an `Err`, so the
  handler's fallback arm does not catch it. That is exactly the multilingual
  corpus a cross-lingual embedder like bge-m3 exists for.
- `chunk_content` is truncated to 200 characters at write time
  (`embedding/handler.rs:280`), so retrieval is document retrieval, not passage
  retrieval, even now that chunk ids resolve.

The one part of the chain that **is** fixed, in this worktree, by a parallel
agent: HNSW results now carry a source `node_id` plus `chunk_id` and
`chunk_index` (`raisin-hnsw/src/types.rs:168-181`) instead of handing
`{node_id}#{i}` to `storage.nodes().get(...)`, and `HYBRID_SEARCH` fuses on
`(workspace_id, node_id)` with RLS applied per hit
(`raisin-sql-execution/src/physical_plan/table_function.rs`). Before that,
enabling chunking deleted the vector arm of every search. Treat the surrounding
findings as still open.

**The order to fix them in**, since only the first is a prerequisite for the
rest: (1) collapse the two config stores to one and point the console at it;
(2) fix the non-ASCII panic; (3) make `chunk_size` mean tokens or rename it;
(4) store the full `chunk_content` and surface `excerpt`; (5) fold the chunking
config into the embedder identity so a config change invalidates.

---

## 7. What I built, and what I only designed

### Built, compiled and tested

Four files in this worktree. `cargo check -p raisin-functions` and
`cargo check -p raisin-server --features "storage-rocksdb,websocket,pgwire"`
are clean; four new unit tests pass.

**`crates/raisin-functions/src/plugin.rs`** — the probe and the stubs.
- `assemble_plugin_js()` is now **always non-empty**. It installs a frozen
  `raisin.capabilities` (`has(method)`, `namespace(ns)`, `plugins`, `methods`),
  built from `registered_plugin_methods()` — the same registry the dispatcher
  reads, serialised through `serde_json` so a name containing a quote cannot
  break out of the generated source.
- For every `OPTIONAL_PLUGIN_NAMESPACES` entry no plugin provides, it installs a
  `Proxy`-over-a-function stub whose every call returns
  `{ ok:false, reason:'plugin_absent', method }`. Nested access works
  (`raisin.media.doc.toPdf`), `typeof … === 'function'` still reports true so
  Studio's existing guard behaves, `await` works (the `then` trap returns
  `undefined`, so it is not treated as a thenable), and a loaded plugin's real
  namespace overwrites the stub because plugin snippets are appended last. The
  semantics were verified by running the generated JS under node before wiring
  it.
- `OPTIONAL_PLUGIN_NAMESPACES` is `["media"]` only. **The stripe plugin is
  deliberately excluded**, and this is the non-obvious part: it installs the
  *nested* `raisin.maravilla.stripe` via
  `raisin.maravilla = raisin.maravilla || {}`. A Proxy stub on `raisin.maravilla`
  would be truthy, survive the `||`, and then shadow the plugin's own `.stripe`
  assignment behind the stub's `get` trap — turning a **working** plugin into a
  soft failure. Only top-level, wholly-owned namespaces may be listed; the
  comment in the code says so.
- `record_plugin_rejection` / `rejected_plugins()` / `plugin_manifest()` /
  `registered_plugin_methods()` / `plugin_method_available()` /
  `plugin_namespace_available()`.

**`crates/raisin-functions/src/plugin_loader.rs`** — a rejected plugin is now
recorded, not only logged.

**`crates/raisin-functions/src/media_capabilities.rs`** *(new)* — the one
`MEDIA_KINDS` table, `capability_report()` resolving each kind against the live
registry into `Plugin | Core | Unsupported`, and `log_capability_report()`
(WARN on `UNSUPPORTED`, WARN per rejected plugin). Its doc comment carries the
code citations for every "core does X" claim so the table cannot drift from the
binary unnoticed.

**`crates/raisin-server/src/main.rs`** — logs the plugin directory before the
scan and calls `log_capability_report()` after it.

Tests: `plugin::tests::probe_and_stubs_exist_without_any_plugin`,
`plugin::tests::manifest_and_method_lookup_agree`,
`media_capabilities::tests::every_kind_resolves_and_core_only_matches_the_code`
(asserts that with no plugin the Core rows are exactly `pdf` and `image` — so
this file lying about the binary fails the build),
`media_capabilities::tests::rendering_names_the_provider`.

### Designed here, deliberately not built

- **`GET /api/management/plugins`** (§5.1) — everything it needs is exported and
  `Serialize`; not built for disk headroom, not for design reasons.
- **The `media_processing` durable trace** (§2.4) — the shape, the enum, the
  backfill query and the write site are specified. Not built because it belongs
  in the asset pipeline that a parallel agent is actively editing in this same
  worktree, and a second writer of the same property is exactly the drift this
  repo punishes.
- **Startup config/reachability probes** (§3.1) — the WARN-on-empty-config and
  ERROR-on-missing-explicit-`RAISIN_PLUGIN_DIR` rules are small and local; the
  Delivery reachability probe needs `GET /_internal/health` on Delivery and a
  `media.health` method on the plugin, both in the owner's repos.
- **Rewriting the two Studio functions** onto `raisin.capabilities` — they live
  in the `studio` repo. The stubs make them safe in the meantime, which is why
  the stub shipped first.
- **Deploy-chain fixes** (§4.2) — all four are in `cloud-ops` and
  `maravilla-runtime`, outside this repo, and step 1 (verify the load) depends
  on §5.1 existing.
- **Everything in §6 beyond the plain answer.** Collapsing the two chunking
  config stores touches the console, two HTTP handlers and a Rust config type;
  it is a change with a migration, not a low-risk slice.

### Two things worth flagging to whoever picks this up

- The `.so` is native code `dlopen`ed into the database process from a directory
  whose contents are the config (there is no allowlist — every `.so`/`.dylib`/
  `.dll` in it is loaded). Steps 3 and 6 of §4 mean the bytes are not pinned.
  That combination deserves the checksum fix more than its "minor" audit label
  suggests.
- `INTERNAL_API_TOKEN` is one value (`vault_muli_api_key`) shared by raisindb,
  Delivery, flightdeck and the Caddy tenant-map unit. The media plugin's
  credential is therefore usable against flightdeck's internal API and vice
  versa. Giving the media/storage surface its own token is a small change with a
  large blast-radius reduction.
