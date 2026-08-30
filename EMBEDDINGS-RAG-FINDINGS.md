# Embeddings & RAG — what actually works

**Yes, embedding works. It was proven live, twice, by two independent agents, on a real
server against real ollama `nomic-embed-text` (768d) — writes go in through plain SQL
`INSERT`, REST and WebSocket, and `ORDER BY embedding <=> EMBEDDING('...') LIMIT k`
returns semantically correct, HNSW-backed, RLS-filtered rows with real cosine distances.**

**But it did not work out of the box.** On the committed tree (`0688af45`) the HNSW index
was built at a hardcoded 1536 dims, so every 768-dim vector was silently rejected and
search returned nothing with no error anywhere. That, plus five other blockers, were fixed
in this worktree — **74 modified files, 11 new, +4316/−1166 lines, all uncommitted.**

**What is still missing is the front half.** Upload → preview/thumbnail → extract → chunk →
embed is now partly real (PDF text extraction landed), but thumbnails/previews do not exist
in raisindb at all, chunking is opt-in and misconfigured by default, there is no
programmatic/flow API to write a vector, and no Studio pipeline or agent tool has been built.

---

## 1. Reproduce it in five minutes

Prereqs: ollama running (`ollama serve`, `ollama pull nomic-embed-text`), a release
`raisin-server` built from **this worktree** (`SKIP_ADMIN_BUILD=1 cargo build --release -p
raisin-server` — see §5.6 on build), a fresh data dir.

```bash
# 1. Start (outside --dev-mode these three env vars are MANDATORY, see §5.9)
JWT_SECRET=dev-only-not-a-real-secret \
RAISINDB_SIGNING_SECRET=dev-only-not-a-real-secret \
RAISIN_MASTER_KEY=$(python3 -c "print('11'*32)") \
./target/release/raisin-server --config rag.toml   # port 8099, repo `rag`

SQ() { curl -s -X POST http://127.0.0.1:8099/api/sql/rag \
  -H 'Content-Type: application/json' -H "Authorization: Bearer $TOKEN" \
  -d "{\"query\":$(jq -Rs . <<<"$1")}"; }
```

```sql
-- 2. Declare a vector-indexed property. Both spellings now parse (§5.8):
CREATE NODETYPE proof:Doc (body String REQUIRED VECTOR FULLTEXT);

-- 3. Point the tenant at ollama. No API key needed any more (§5.2):
ALTER EMBEDDING CONFIG
  SET PROVIDER='ollama' SET MODEL='nomic-embed-text' SET DIMENSIONS=768
  SET BASE_URL='http://127.0.0.1:11434'
  SET INCLUDE_NAME='false' SET INCLUDE_PATH='false' SET ENABLED='true';
TEST EMBEDDING CONNECTION;
-- -> {"result":"Connection successful","dimensions":768,"model":"nomic-embed-text"}

-- 4. NO `REBUILD VECTOR INDEX` NEEDED any more. Confirm the width was resolved
--    from your config, before any write:
SHOW VECTOR INDEX HEALTH;   -- {"status":"available","count":0,"dimensions":768}

-- 5. Write, through the surface a user actually writes:
INSERT INTO 'default' (path,name,node_type,properties) VALUES
 ('/d1','d1','proof:Doc','{"body":"Applying the brake pedal presses friction pads against the discs and brings the vehicle to a halt."}'),
 ('/d2','d2','proof:Doc','{"body":"Car soap, a two-bucket rinse and a microfiber cloth keep the paint bright and swirl-free."}'),
 ('/d4','d4','proof:Doc','{"body":"Tamp the grounds level, lock in the portafilter and pull a thirty-second shot."}');

-- 6. THE PROOF. The query shares zero content words with the winner:
SELECT name, embedding <=> EMBEDDING('bringing an automobile safely to a standstill') AS distance
FROM 'default' ORDER BY distance LIMIT 3;
-- -> d1 0.29926612973213196 | d2 0.4064962565898895 | ...
--    d1 wins on brake/pedal/friction/discs/vehicle/halt.
--    d2 — the ONLY doc containing "Car" — ranks below it.

-- 7. THE CONTROL. Prove no lexical route to that answer exists:
SELECT name FROM 'default' WHERE FULLTEXT_MATCH('automobile','en');   -- 0 rows
SELECT name FROM 'default' WHERE FULLTEXT_MATCH('portafilter','en');  -- ["d4"]  (index is live)
SELECT name FROM 'default' WHERE FULLTEXT_MATCH('zzqqxx','en');       -- 0 rows  (not matching everything)

-- 8. Scoping — the thing a RAG retriever needs, fixed in §5.3:
SELECT name FROM 'default' WHERE FULLTEXT_MATCH('portafilter','en') AND node_type='raisin:Folder'; -- 0 rows
SELECT name, embedding <=> EMBEDDING('how do I stop my car') AS d
  FROM 'default' WHERE path='/d2' ORDER BY d LIMIT 3;   -- d2 only
```

`0.29926612973213196` is byte-identical across five independent runs by three agents, on two
data dirs, before and after restart, and via SQL / REST / WebSocket writes. That number is the
single most load-bearing fact in this document.

`INCLUDE_NAME`/`INCLUDE_PATH=false` was verified honoured from the server log — the embedded
text is `text_length=98`, the body exactly, with no name and no path, so no filename could
carry the match.

---

## 2. Status by capability

| Capability | Status |
|---|---|
| Vector write path (node event → job → provider → `cf::EMBEDDINGS` + HNSW) | **Proven working** |
| Vector read path (`<=>` → VectorScan → HNSW → node fetch → RLS) | **Proven working** |
| Write funnel is transport-agnostic (SQL DML, REST, WebSocket) | **Proven working** — `transaction/commit/mod.rs:277` |
| Fulltext (Tantivy) `FULLTEXT_MATCH` | **Proven working** (English only — see §7) |
| Scoping predicates on both retrieval surfaces | **Fixed + proven** (§5.3) |
| `HYBRID_SEARCH(q, k, workspace)` RRF fusion | **Works** — the 2-arg form leaks other workspaces (§4) |
| Embeddings configured via nodetype (`VECTOR` keyword) | **Proven working** |
| Restart survival | **Works with a 60-second hole** (§4) |
| PDF upload → text extraction → fulltext + vector | **Fixed + proven** (§5.4) — PDF only |
| Chunking | **Works but misconfigured by default** (§7) |
| Programmatic / flow API to create an embedding | **Does not exist** (§7) |
| Previews / thumbnails | **Does not exist in raisindb at all** (§7) |
| Multi-node clusters | **Broken** — embeddings are never replicated (§7) |
| Studio pipeline + Test Chat agent tool | **Designed, not built** (§8) |

---

## 3. Where a sceptic should look first

Everything in this section was re-verified against the repo while writing this document.

The engine is real: `crates/raisin-hnsw/src/index.rs:328` delegates to `usearch::Index::search`.
`EXPLAIN` on the headline query prints `VectorScan: table=default, column=embedding, k=3,
metric=Cosine` — no table scan, no post-hoc sort. Querying with a document's *exact* body
returns it at distance `3.39e-21` and everything else above `0.53`, so the stored vectors are
real, not zeros or random. Pointing `BASE_URL` at a dead port makes the query fail against that
port, so `EMBEDDING()` is a live round-trip to the model at query time, not a cached constant.
The on-disk sidecar reads `{"dimensions":768,"distance_metric":"Cosine","quantization":"F32"}`.

Isolation was tested the hard way: a near-duplicate document planted in another workspace with
a *better* distance (0.2551 vs 0.3450) never appeared in the scoped query, and a better match in
another repo likewise never appeared. The index file layout is
`hnsw_indexes/<tenant>/<repo>/<branch>.hnsw`, so tenant partitioning is structural rather than a
filter that could be forgotten. (Cross-*tenant* could not be tested live — `POST /api/tenants`
404s in this build.)

Ranking is query-dependent, not a fixed order: an independent 10-document corpus with 7 queries
of the verifier's own returned the correct target at rank 1 for all 7 across 6 distinct targets.

---

## 4. What the independent verifier knocked down

These override the proof transcript. Do not believe the transcript where it disagrees.

1. **"`REBUILD VECTOR INDEX` before the first write is mandatory" — REFUTED on the current
   tree.** The trap was real at `0688af45` and is closed (§5.1). The verifier never ran REBUILD
   and every insert indexed on the first try.
2. **"`SHOW VECTOR INDEX HEALTH` reports 1536 forever" — REFUTED.** It reports the loaded
   index's real width now.
3. **"Fulltext / vector silently drop scoping predicates" — REFUTED on the current tree**
   (real at HEAD, fixed in §5.3).
4. **"The read path drops `base_url`" — REFUTED** (fixed in §5.2).
5. **NEW, and not in the transcript: a restart loses up to 60 seconds of vectors.**
   `crates/raisin-hnsw/src/engine/lifecycle.rs:24` snapshots every 60s; nothing WAL-protects the
   interval and `shutdown()` has no caller. A node written and restarted immediately came back
   **silently unsearchable** — `VERIFY VECTOR INDEX` said `hnsw_count=10, storage_count=11`, the
   node and its stored vector were both fine, and search just omitted it, with no error.
   `REBUILD VECTOR INDEX` recovered it at the identical distance. **This is a real production
   bug and it is not fixed.** The proof run passed only because its writes were minutes old.
6. **`HYBRID_SEARCH(q, k)` — the 2-argument form — is cross-workspace by design**
   (`table_function.rs:452-463`). On a real repo it returned rows from workspaces `other`,
   `functions` and `packages` above the correct answer. Pass the undocumented 3rd argument. The
   proof run missed this because its repo had one content workspace.
7. **Methodological**: the proof's binary-faithfulness check used `git log --since=`, which
   cannot see uncommitted work — and this worktree has ~1800 uncommitted lines in exactly the
   files in question. In a dirty tree use `git status --short` and `strings` on the artifact.

---

## 5. What was fixed here (all uncommitted, all live-verified)

`git diff --stat` → **74 files changed, 4316 insertions(+), 1166 deletions(-)**, plus 11 new
untracked source files. Every touched crate builds `--release`; `cargo fmt --all --check` exits
0 (which is the entirety of this repo's CI); scoped tests pass: raisin-sql 310, raisin-ai 851,
raisin-sql-execution 821 lib + 179 `--test all`, raisin-rocksdb 1029 lib, raisin-hnsw 38,
raisin-crypto 49, raisin-embeddings 43. 129 assertions added, 0 `#[ignore]` added.

**5.1 HNSW index width is no longer a startup constant.**
`crates/raisin-server/src/startup/indexing.rs:62` now passes `raisin_hnsw::FALLBACK_DIMENSIONS`
plus a `TenantEmbeddingDimsResolver`, and `get_or_load_index` resolves the tenant's width on the
cache-miss path from the same `TenantEmbeddingConfig` row the job handler reads. New
`crates/raisin-hnsw/src/dims.rs`. `stats()` reports the *index's* width, not the engine constant
— that was the second half of the bug, because the one diagnostic an operator reaches for was
lying. A populated index whose width disagrees now hard-errors naming `REBUILD VECTOR INDEX`;
an empty one self-heals. Before: every 768-dim vector failed inside a background job with
`Vector dimension mismatch: expected 1536, got 768`, three silent retries, health `count:0`.

**5.2 Six drifted provider resolvers collapsed into one.** New
`crates/raisin-embeddings/src/resolve.rs` returns a *built* provider (not a tuple of parts, so
`base_url`/`dimensions` cannot be dropped by a caller again), backed by
`crates/raisin-rocksdb/src/embedding_provider.rs`. The job handler's private resolver and
`QueryEngine::decrypt_api_key` were **deleted** so they cannot be picked up again.
`requires_api_key(base_url)` is now asked once, of the provider variant, which removes the
`SET API_KEY='unused-but-required'` hack the original proof needed. Measured before/after on one
config: `POST /config/test` said `success:true, 768` while the SQL test said "No API key
configured" and 16 embedding jobs failed in the log — the green light was the lie. After: both
green, `count:6`, zero failures. With `ai_provider_ref` set (what the console writes) the
pre-fix engine ignored it and 401'd against OpenAI; after, it reaches ollama.

**5.3 Both retrieval surfaces keep their scoping predicates.** New
`PhysicalPlanner::wrap_with_residual` in `scan_planning/mod.rs` is now the one place a scan
acquires a residual `Filter`; FullTextScan, NodeIdScan, PathIndexScan and VectorScan all use it.
The vector bug was worse than reported: `vector_knn.rs` read the filter only from
`Filter{Scan}`, a shape that exists only when pushdown *declined* — in the normal case the
predicate sits in `Scan.filter`, a field it never read, so the guard that was supposed to
protect correctness never ran. `is_simple_predicate` (a performance judgement wearing a
correctness hat) was deleted. `VectorScan` gained `overfetch` (k×20 when a residual is present,
1 otherwise) so a scoped `LIMIT 1` cannot find its one global neighbour, reject it, and answer
empty. Measured: `WHERE node_type='raisin:Folder' ORDER BY embedding <=> …` returned two
`proof:Doc` rows before, zero after; `WHERE path='/d6' AND FULLTEXT_MATCH('mainsail')` returned
2 rows before, the correct 1 after.

**5.4 PDF upload → extraction → index chain reconnected.** `PdfProcessor::process`
(`raisin-ai/src/pdf/router.rs:155`) was an unconditional `Err(NotAvailable)` stub; it now calls
the same `extract_markdown_from_bytes` the storage path uses (a fork deleted, not a copy added).
`raisin_asset.yaml` v3→v4 adds `Vector` to `index_types` (without it *no* `EmbeddingGenerate`
job was ever enqueued for an asset) plus `extracted_text` and `extraction_fingerprint`.
Extraction terminates in a **node property** written through `NodeService::update_node` — so the
existing commit → `emit_node_events` → fulltext+embedding funnel picks it up with no new
plumbing, and replication carries the text so a replica never redoes the work. The write-back's
own `node:updated` is caught by the fingerprint gate
(`v1:{content_hash}|{storage_key}|{file_size}`) so it cannot loop; replacing the file re-opens
it. Measured on 4 PDFs: before → all `extracted_text` null, `FULLTEXT_MATCH('portafilter')` 0
rows, `SHOW VECTOR INDEX HEALTH count:0`, and the job logged "PDF processing not available" then
reported **success**; after → all four texts stored, count:4, `brakes.pdf` at
0.29926612973213196 on the headline query.

**5.5 Three `REBUILD` implementations became one, and stopped guessing the workspace.** Each
named a *different* hardcoded workspace — `"staff"` in `management/vector.rs` and
`vector_embeddings.rs`, `"default"` in `ai_config.rs` — while the write path indexes whatever
workspace the node lives in. Every admin vector operation was therefore a **silent no-op
reporting success** on any deployment whose content sits elsewhere. New
`EmbeddingStorage::list_workspaces` (prefix scan over the existing key layout) replaces the
guess; SQL delegates to `HnswManagement`; `items_processed` now counts what was *added*, not
what was *listed* — which was the "rebuilt with 6 embeddings" / `count:0` lie.

**5.6 / 5.7 Build unblocked.** `crates/raisin-server/build.rs` had five nested-cargo spawn sites
scrubbed three different ways; a nested cargo inheriting `CARGO_TARGET_DIR` deadlocks on the
artifact flock its own parent holds (confirmed by `lsof`: two cargo PIDs on one inode, no rustc
running). One `nested_build_command` helper now covers all five. Separately,
`pnpm-workspace.yaml` needed `allowBuilds: esbuild: true` — pnpm ≥10 exits 1 rather than
skipping a non-allowlisted lifecycle script and `build.rs` `.expect()`s on it, so a missing
one-line config was an outright build failure behind a message naming neither pnpm nor esbuild.
`cargo build --release -p raisin-server` now works with the admin console included.

**5.8 DDL front door.** `CREATE NODETYPE proof:Doc (body String REQUIRED VECTOR FULLTEXT)` was
rejected with `Invalid property type 'CREATE'` at position 0 — because `ddl_statement` was a
12-arm `alt()` and nom reports the *last* arm's failure, so a broken CREATE was diagnosed by the
`DROP ELEMENTTYPE` parser. That message names a real category and a real token, so it reads like
a fact; it cost the first proof run a wrong turn. One keyword table now both recognises and
dispatches, so exactly one parser runs and its error is the one reported. Bare names and the
implicit property block now parse across all twelve statement forms.

**5.9 Production boot.** Three serial `exit(1)` blocks became one preflight
(`crates/raisin-crypto/src/env_secrets.rs`) reporting **every** unmet requirement in one pass —
and the master-key check now validates *through* `master_key_with_embedding_fallback()` instead
of testing variable *names*, which fixes a real failure where a fully-configured
`RAISIN_MASTER_KEYS` keyring deployment could not boot at all, and makes a malformed key fail at
boot instead of panicking the job system later. Worth folding into CLAUDE.md: outside
`--dev-mode` a node needs `JWT_SECRET`, `RAISINDB_SIGNING_SECRET`, and either
`RAISIN_MASTER_KEY=<64 hex>` or `RAISIN_MASTER_KEYS` + `RAISIN_MASTER_KEY_ACTIVE`. No config
file supplies them.

**5.10 RLS on the search table functions.** `FULLTEXT_SEARCH` and `HYBRID_SEARCH` previously
emitted each hit's **complete property bag with no permission check**, and HYBRID_SEARCH fetched
from a hardcoded `"default"` workspace so it could permission-check against the wrong scope.
Both now run through the same `rls_filter_node_graph` every scan executor uses.

---

## 6. Review risks before this lands

- **Undisclosed scope.** Beyond embeddings this diff also contains a DDL-parser rewrite
  (~350 lines), an asset-processing pipeline with task routing and per-rule chunking, plugin
  capability reporting, and a secrets preflight. Every piece is coherent, formatted and green,
  but reviewing it as "an embeddings proof" would under-scrutinise it.
- **Breaking change, undocumented**: `HYBRID_SEARCH` now hard-errors when an HNSW engine is
  present but the tenant has no resolvable embedding config. It used to degrade silently to
  fulltext with NULL vector columns. Fail-closed is right, but existing callers will start
  erroring on upgrade.
- **Cost event**: `raisin_asset.yaml` v3→v4 ships via the system-definitions OTA mechanism.
  After resync every existing `raisin:Asset` begins enqueuing `EmbeddingGenerate` on its next
  update. On a paid embedding API that is real spend. Put it in the release notes rather than
  discovering it from the bill.
- **New production default**: `default_model_for_kind(Ollama) => "nomic-embed-text"`
  (`resolve.rs:280`). Fires only when a tenant points `ai_provider_ref` at Ollama with no model
  ref — but it is a specific model name baked into a production path by the session proving
  against that model. Worth a deliberate decision.
- **Dead code left standing**: `crates/raisin-server/src/embedding_worker.rs`,
  `embedding_worker/job_handlers.rs` and `embedding_event_handler.rs` appear in no `mod` list
  and do not compile in (verified: `grep -rn "mod embedding_worker" crates/raisin-server/src`
  is empty). Two contain the old `create_provider` base-URL bug and one writes **unnormalized**
  vectors. They also make `cf::EMBEDDING_JOBS` permanently empty. Delete them.
- **Verified nits**: `ai_config.rs:527` still hardcodes `list_embeddings(..., "default")` — a
  surviving instance of exactly the bug class fixed 85 lines above (informational count only).
  `management/vector.rs`'s new purge loop iterates workspaces but `purge_index` ignores its
  `_workspace_id` parameter, so it purges the same index N times, and purges **nothing** when
  the workspace list is empty despite a comment promising the opposite.
  `raisin-sql-execution/Cargo.toml` adds `async-trait = "0.1"` as a literal instead of
  `{ workspace = true }`. Two untracked docs sit at the repo root
  (`CORE-PLUGIN-DEPLOYMENT.md`, `MULTILINGUAL-SEARCH-ANSWER.md`), one cited from a code comment
  in `main.rs`.

---

## 7. Still broken / missing

All verified against the repo in its current (uncommitted) state.

**Blocking for a multi-node deployment**
- **Embeddings are never replicated.** `grep -ri embedding crates/raisin-replication/src`
  returns **zero hits**, and replicated nodes are skipped by the embedding enqueue
  (`node_handlers.rs:139-146`). On a cluster the vector index permanently diverges while
  fulltext converges. The code comment asserting a mechanism exists is false.
- **The 60-second snapshot hole** (§4.5). `lifecycle.rs:24`; `shutdown()` has no caller; and a
  dirty index evicted from the LRU cache loses its unsaved writes (`lifecycle.rs:70-73`).

**Blocking for RAG anywhere but `/api/sql`**
- **Only HTTP `/api/sql` declares the `embedding` column.** `sql/engine.rs:34` is the sole
  caller of `workspace_catalog_with_embeddings`. pgwire (`extended_query/handler.rs:154`,
  `simple_query/execution.rs:41`), WebSocket (`nodes/sql_query.rs:72`) and function SQL
  (`callbacks/sql.rs:54`) all call plain `workspace_catalog`, so `EMBEDDING()` fails there with
  a message pointing at config the user already set correctly.
- **No programmatic / flow authoring route.** The complete embeddings route table is config,
  config/test and bulk regenerate (`routes/admin.rs:39,44,187`). `raisin.ai.embed()` returns a
  float array and persists nothing. There is no `raisin.embeddings.*` binding, no WS request
  type, no flow `StepType` that can persist a vector. The nodetype-declared route is the only
  route. **This is a feature to build, not a bug to fix.**
- **`REGENERATE EMBEDDINGS` is a message stub** that queues nothing (`ai_config.rs:507`), and
  there is **no backfill anywhere**: enabling embeddings on an existing repo embeds nothing
  until every node is written again.

**Correctness and cost**
- **No re-embed suppression.** `text_hash` is written (`jobs/handlers/embedding/handler.rs:289`)
  and never read, so every update — including bookkeeping-adjacent ones — issues a fresh paid
  provider call for byte-identical text.
- **The dedup key omits the revision** (`job_type_methods.rs:43`: `embedding_gen:{node_id}`), so
  a second edit while the first job is running is silently dropped and the stored vector
  reflects stale text with nothing scheduled to correct it.
- **`cf::EMBEDDINGS` grows unboundedly.** `handle_delete` removes vectors from HNSW but
  **`delete_embedding` has no production caller** (verified: only the trait definition and
  tests), and `store_embedding` writes a new key per revision. `REBUILD` can therefore resurrect
  deleted nodes' vectors. Same shape as the spatial-index growth already logged in
  `docs/OPEN-ITEMS.md` §2.99.
- **The enqueue gate reads the RAW NodeType** (`index_helpers.rs:112-164`) while the plan
  resolver reads the RESOLVED one (`plan.rs:118-129`), so mixin/supertype-inherited index config
  is honoured in one and ignored in the other. The gate should call the resolver.
- **A resolved NodeType with no `VECTOR` property embeds name+path only, silently** — the text
  is non-empty so the empty-text guard never fires, and semantic search returns confident
  nonsense with only a `trace!` line explaining it.
- An implicit **0.6 cosine cutoff** (`engine/search.rs:101-104`) silently makes `LIMIT k` return
  fewer than k rows, and the tenant's own `default_max_distance` never reaches the search. `k`
  is also truncated *before* workspace filtering, RLS and locale expansion, so a restricted user
  gets an arbitrarily short result set with no top-up loop.
- **The SQL distance metric is never reconciled with the metric the index was built with** —
  `VECTOR_L2_DISTANCE` on a cosine index returns cosine distances labelled L2, and the tenant's
  `DISTANCE_METRIC` setting reaches no index. Row-level cosine on the TopN fallback also does
  not normalize the query vector and is implemented twice (`eval/binary_ops.rs:154`,
  `eval/async_eval.rs:186`), so a `d < 0.4` threshold means different things on the two plan
  shapes for the same query.
- **Multi-model is stored but not selectable**: the HNSW key is only
  `{tenant}/{repo}/{branch}`, `add_embedding` carries no embedder, and `get_embedding` returns
  the first `source_id` match whatever wrote it.

**Chunking**
- `chunk_size` is documented as tokens and passed to a `Characters` sizer, and the `splitter`
  setting is never read — so the default config yields ~256-character chunks, 4× smaller than
  intended, and changing the strategy does not change the embedder hash.
- The `{node_id}#{i}` / source-id split *is* now handled correctly
  (`raisin-hnsw/src/types.rs:201`, `parse_chunk_id`, which fails closed on a non-numeric
  suffix), so the "enabling chunking returns zero rows" defect is **closed**. But per-chunk
  character offsets are not stored, so an excerpt slicer has to approximate.

**Fulltext**
- **Language stemming is dead.** `{lang}_stemmer` analyzers are registered but the name and
  content fields hardcode `.set_tokenizer("default")`
  (`raisin-indexer/src/tantivy_engine/schema.rs:33`). `databases` never matches `database`, and
  non-English repos get English tokenisation.
- The fulltext **rebuild hardcodes language `"en"`** (`management/fulltext.rs:49`) while the
  live path reads the repo's `default_language` — so an operator rebuild on a `de` repo makes
  `FULLTEXT_MATCH(...,'de')` return zero rows. Rebuild is the recovery action operators reach
  for. Documented examples also use PostgreSQL language names (`'english'`), which match zero
  documents because the language clause is a hard MUST against the ISO code.
- `FULLTEXT_MATCH` **hardcodes a 1000-hit fetch regardless of `LIMIT`**
  (`predicate_ops.rs:43`): a `LIMIT 10` query does 1000 node reads plus 1000 RLS graph checks,
  recall caps at 1000, and there is no paging.
- `POST /fulltext/search` and `GET /api/search` still bypass RLS (only the SQL surfaces were
  fixed in §5.10). The vector/fulltext management endpoints (verify/rebuild/regenerate/optimize/
  restore/health) carry **no auth layer at all** — the router's own comment says so.
- `KNN()` is registered, documented, autocompleted and taught in
  `docs/website/docs/why/overview.md:211`, and the executor rejects it as "Unsupported table
  function". pgvector's `<->` / `<#>` cannot be parsed (`RaisinDialect` does not override
  `is_custom_operator_part`), which makes the `op_str.contains("<->")` fallbacks in the analyzer
  read as if they work.
- `VectorScan` discards the `Project` it replaced, so
  `SELECT properties->>'title' AS title, … ORDER BY d LIMIT 5` can lose the title column
  depending on plan shape.

**Files and uploads**
- **No thumbnail or preview generation exists anywhere in raisindb.** `pdf_to_image` was
  deliberately removed. The Maravilla media plugin supplies every missing transform but is
  guest-JS-only and needs `DELIVERY_MEDIA_URL` + `INTERNAL_API_TOKEN`; neither is set on this
  machine, so **the delegated half was never demonstrated end to end**.
- `is_extractable_mime` matches `application/pdf` and nothing else
  (`asset_processing/helpers.rs:225`). `.txt/.md/.html/.csv` are pure Rust and belong there —
  but widening it without adding a `process_extractable` branch gives a job that reports success
  and stores nothing.
- Storage keys are date/nanoid-based, not content-addressed, and the upload path never populates
  `raisin:Asset.content_hash` — so re-uploading identical bytes yields a new key, a new
  fingerprint, and a full re-extract plus re-embed.

**Studio**
- No trigger fires on asset upload. No shipped agent declares any tools. The only working
  PDF→text pipeline is walled inside the QLoRA feature, hardcoded to the `models` workspace, and
  its own `.node.yaml` forbids invoking it from a trigger. Studio has zero semantic-search code
  and three hand-copied `FULLTEXT_SEARCH` call sites — a fourth would repeat the drift pattern,
  so the search API should land as one shared helper. `raisin:Asset` is a platform nodetype
  Studio cannot extend from its package.
- **`raisindb deploy --install` does not update existing `raisin:AIAgent` / `raisin:Flow` /
  function bodies.** Any proof run in Studio must use `raisindb sync . --push` and verify by
  GET, not trust the deploy output.

---

## 8. Designed, not built: the Studio pipeline

Design doc: `scratchpad/DESIGN-studio-asset-pipeline.md`.

The architecture is decided by a hard constraint, verified not assumed: `raisin-functions`
depends on `raisin-rocksdb`, and Cargo rejects package cycles regardless of features. The plugin
registry lives in `raisin-functions`, so **the AssetProcessing job can never call
`raisin.media.*`** — this is structural, not a TODO. The pipeline is therefore two halves
meeting at a node property: a **job** does native extraction (needs raw bytes, must survive
restart, parses attacker-controlled input); a **flow** does media transforms (media calls are
submit-only, so it must park — a `wait` step re-armed via `raisin.scheduler.schedule` with an
`externalKey`, the pattern the maravilla-media companion package already uses). That handoff
being a durable node property is also what gives failure isolation: LibreOffice cannot fail the
embedding because it is not in that call stack.

The routing table already existed and was extended rather than duplicated: `ProcessingRuleSet`
(`raisin-ai/src/rules/`, persisted per-repo in `CF_PROCESSING_RULES`, served over
`/api/repository/{repo}/ai/rules`, edited in the admin console). Two real bugs were fixed there:
the matcher compared MIME types with `==` while the *shipped default image rule* was the string
`"image/"`, a value no upload can carry — so **image processing matched nothing in every
installation** and `ProcessingSettings::image()` was unreachable code; and
`ProcessingSettings.chunking` was persisted, HTTP-editable, round-trip tested and **read by
nothing**. Per-rule chunking now reaches the embedder, demonstrated live taking one 753-character
document from 1 embedding to 23. Blast radius is safe: `default_rules()` has no production
caller, so a fresh install falls to the preserved mime defaults and behaviour is unchanged.

**What was not built:** the entire media flow (no trigger, no flow definition, none of the
stage→submit→wait→collect→attach functions), multi-page previews, a `thumbnail` /
`preview_pages` property on `raisin:Asset` (needs a platform nodetype bump — Studio's package
cannot add it), the `search_documents` agent function and its wiring, and the excerpt slicer.

---

## 9. What to do next — shortest path to value first

1. **Decide on this branch.** 74 files of uncommitted work spanning six subsystems. Split it:
   (a) embeddings/HNSW/provider/scoping, (b) DDL parser, (c) asset pipeline, (d) build +
   secrets. Land (a) and (d) first — (a) is what makes the feature work at all, (d) is what
   makes a production node boot.
2. **Fix the 60-second snapshot hole** (§4.5). It is the only *silent data loss* left, it hits
   every restart, and `shutdown()` already exists with no caller. Half a day.
3. **Delete the three dead embedding modules** before someone revives the one that writes
   unnormalized vectors. An hour, and it also un-mysteries the permanently-empty
   `cf::EMBEDDING_JOBS`.
4. **Declare the `embedding` column in the other three catalog call sites** (pgwire ×2, WS,
   functions). One shared helper, ~20 lines. This is the difference between "RAG works in curl"
   and "RAG works from a function, a flow, or psql".
5. **Read `text_hash` before calling the provider**, and put the revision in the dedup key. Two
   small changes that stop paying per-token for byte-identical text and stop dropping concurrent
   edits.
6. **Fix the tokenizer** (`tantivy_engine/schema.rs:33`) and the rebuild's hardcoded `"en"`.
   Fulltext is the more mature half and is currently English-only by accident.
7. **Build the programmatic route**: `raisin.embeddings.upsert()` plus a flow `StepType`. This
   is the "created programmatically / by a flow" ask, and nothing exists for it today.
8. **Then the Studio pipeline**, in this order: the `search_documents` function + agent tool
   (§8 has the exact input schema and the `HYBRID_SEARCH(query, limit, workspace)` SQL — this
   alone gives a working Test Chat over already-indexed content), then the upload trigger, then
   the media flow, then thumbnails.
9. **Before any cluster deployment**: replicate embeddings, or accept that vector search is
   single-node and say so in the docs.
10. **Widen `is_extractable_mime`** to `.txt/.md/.html/.csv` — pure Rust, no new dependency, and
    it multiplies what the RAG corpus can contain. Add the `process_extractable` branch in the
    same commit or the job reports success and stores nothing.

---

### Appendix: where the artifacts live

- Full proof transcript, all sections: `scratchpad/PROOF-embeddings.md`
- Studio pipeline design: `scratchpad/DESIGN-studio-asset-pipeline.md`
- Runnable harnesses: `scratchpad/proof/`, `scratchpad/faithful/`, `scratchpad/verify-dims/`,
  `scratchpad/unify/`, `scratchpad/pdfproof/` — each with `start.sh`, `rag.toml`, `sq.sh`,
  `corpus.sh`, `battery.sh`, `server.log`
- Independent adversarial verification (own corpus, own queries):
  `scratchpad/adv-verify-8117/`

*(Scratchpad root:
`/private/tmp/claude-501/-Users-senol-Projects-maravilla-labs-repos-clean-raisindb/68c7207c-a347-4cb3-bdef-70d9efd7f6ac/scratchpad/`)*

*Disk note: this machine was at 99% during the work. Always
`export CARGO_TARGET_DIR=<repo>/target`, always scope builds to `-p <crate>`, never run
`--workspace`, and use `SKIP_ADMIN_BUILD=1` unless you need the admin console.*
