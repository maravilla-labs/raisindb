# Multi-language in RaisinDB search: what happens today, and what must happen

Scope: the RAG path — upload → extract → chunk → embed → semantic + fulltext search → agent query —
on a multi-tenant install.

Read in the worktree `.claude/worktrees/rag-embeddings-proof` at commit `0688af45`. Line numbers are
from that tree. This revision folds in two independent challenge passes; every claim they refuted has
been re-read at the cited lines and corrected here, and every point they raised as missed is either
folded in or explicitly dismissed.

Caveat: a concurrent workflow is editing this worktree (`startup/indexing.rs`, `raisin-hnsw/`, a new
`raisin-hnsw/src/dims.rs`). All citations below are to the **committed** tree at `0688af45`; some may
already be stale in the working copy — notably change #19, which that workflow appears to be building.

Each major claim carries a confidence tag: **[verified]** = I opened the file at the cited lines;
**[inferred]** = follows from verified code but I did not execute it; **[unverified]** = stated but
not proven here.

---

## 1. The short answer

A node with English base content and a German translation gets **exactly one embedding vector, built
from the English text**; the German overlay is never read by the embedding job and is therefore
**invisible to semantic search**. In fulltext it gets one Tantivy document per repo-configured
supported language *if and only if* the legacy inline `node.translations` map is populated — which a
live HTTP endpoint does write — and every one of those documents carries **byte-identical
base-language `content`**; only `name` can differ. There is **no stemming in any language**, because
the 16 language analyzers are registered under names no schema field references. The query side is
**prefix-fuzzy with edit distance 1**, which masks some of that but is itself an undocumented
recall/precision decision. CJK is worse than unstemmed: `RemoveLongFilter(40 bytes)` **deletes** any
unspaced run of ~14+ characters at index time.

**What to do**: ship the bug fixes (Phase 0) and per-language Tantivy analyzers (Phase 1) with a
*real, written* migration; add the model-multilinguality flag and replica translation apply; then
**measure cross-lingual recall** and only then decide whether per-locale embeddings are needed at
all. Per-locale embeddings (Option C) as originally drafted are **not safe to adopt**: three of their
load-bearing premises are refuted by the code, and their memory and money costs were never computed.

---

## 2. Today's actual behaviour, for one node with EN base + DE translation

Setup: `default_language = "en"`, `supported_languages = ["en", "de"]`
(`crates/raisin-context/src/repository/config.rs:22,27`, defaults `:60-66`). DE text written through
`TranslationService::update_translation` or `UPDATE ws FOR LOCALE 'de' SET ...` lands in
`cf::TRANSLATION_DATA` (`crates/raisin-rocksdb/src/lib.rs:241`), keyed
`{tenant}\0{repo}\0{branch}\0{ws}\0"translations"\0{node_id}\0{locale}\0{~rev:16B}`
(`crates/raisin-rocksdb/src/repositories/translations/keys.rs:29-46`), as a sparse
`LocaleOverlay::Properties { data: HashMap<JsonPointer, PropertyValue> }`
(`crates/raisin-models/src/translations/types/locale_overlay.rs:76-84`). **[verified]**

**The DE text is not in the node blob.** That single fact drives everything below.

### 2a. What text the embedding job extracts, and how many vectors are stored

**One vector. Base language only.** **[verified]**

- The enqueue gate is **not** what the earlier draft said. `node_handlers.rs:139-147` wraps the whole
  embedding arm in `if !is_remote_event { … } else { debug!("Skipping vector embedding for
  replicated event (embeddings are replicated separately)") }`. The real condition is
  `!is_remote_event && embeddings_enabled(tenant) && index_settings.vector`. **[verified]**
- That comment is wrong: **embeddings are not replicated separately**.
  `grep -n embedding crates/raisin-replication/src/operation/op_type.rs` returns zero hits — no
  `OpType` carries a vector. A replica generates none locally and receives none over the oplog;
  `cf::EMBEDDINGS` arrives only via checkpoint/SST ingest or `JobType::EmbeddingBranchCopy`
  (`event_handler/repo_handlers.rs:68-69`). **[verified]**
- **Fulltext, by contrast, runs for remote events too** (`node_handlers.rs:71-72`, explicitly
  commented "runs for BOTH local and remote events"). That is a fifth, previously unnamed divergence
  between the two halves of search — see §5. **[verified]**
- Writing the DE translation **emits no event at all**.
  `translation_service/operations.rs` contains zero `event`/`publish`/`Event` tokens (only three
  revision-allocation comments at `:45,:304,:367`). On the transaction side `emit_node_events`
  (`transaction/commit/events.rs:103-110`) takes only `changed_nodes`; `commit/mod.rs:264-273` passes
  only `&changed_nodes`, while `changed_translations` goes to the TreeSnapshot job. **No embedding
  and no fulltext job is ever enqueued for a translation.** **[verified]**
  - *Nuance the earlier draft missed:* translations are invisible to the **event bus**, not to
    everything. `build_node_change_infos` (`transaction/commit/extract.rs:73-79`) pushes each changed
    translation into `RevisionMeta` as `NodeChangeInfo { translation_locale: Some(locale) }`. A
    cheaper first step than a new event system may be to fan out from the change infos that already
    exist. **[verified]**
- When a job does run, it fetches the raw node
  (`jobs/handlers/embedding/handler.rs:124-138`), and `extract_embeddable_content`
  (`embedding/content_extraction.rs:31-77`) reads `node.name` (`:41-43`), `node.path` (`:46-48`) and a
  plan-driven walk of `node.properties` (`:72`). Grep for `TranslationResolver` /
  `resolve_node_for_locale` / `translations()` across `jobs/handlers/` returns **zero**; grep for
  `locale|language|lang_code` across `raisin-embeddings/src`, `raisin-hnsw/src` and
  `jobs/handlers/embedding` returns **zero**. **[verified]**
- One vector is stored, keyed
  `{tenant}\0{repo}\0{branch}\0{ws}\0{embedder_hash}\0{kind}\0{source_id}\0{chunk:04}\0{~rev:16B}`
  (`repositories/embedding_storage/storage.rs:42-75`; `encode_descending() -> [u8;16]` at
  `raisin-hlc/src/lib.rs:132-144`). **No locale segment.** `EmbedderId` carries
  provider/model/dimensions/tokenizer_id and no locale, and `tokenizer_id` is hardcoded `None` at the
  one construction site (`raisin-ai/src/config/embedder.rs:30-37`, used at `handler.rs:182-186`), so
  it never discriminates. **[verified]**
- **`store_embedding` ignores its `node_id` parameter.** `storage.rs:214-239` builds the key from
  `data.source_id` and `data.chunk_index`. The handler passes `chunk_node_id` (`{node}#{n}`), but the
  RocksDB row is keyed by the bare `source_id` plus a `{:04}` index — so the `#` suffix never enters a
  RocksDB key. The chunk-id defect in §2e is **HNSW-only**. **[verified]**

`docs/TRANSLATION_EMBEDDING_STRATEGY.md`'s "base language only" policy is **true, but not
implemented**. Its sketched guard `if is_base_language_update(node, base_language)` (`:55`, again at
`:234`) does not exist: `grep -rn "base_language\|is_base_language\|is_translation_update" crates/`
returns nothing. The policy holds by accident of missing wiring, not by a guard. **[verified]**

### 2b. Is the DE text reachable by semantic search at all?

**No.** And on the HNSW side it is structural, not merely unwired: a point's identity is a single
`String` (`raisin-hnsw/src/index.rs:46`) and `add()` removes any prior entry for that string first
(`index.rs:273-279`); the partition key is `format!("{}/{}/{}", tenant, repo, branch)`
(`engine/mod.rs:170-172`). Two locales of one node would collide on one point. **[verified]**

### 2c. What the fulltext index contains, and with which analyzer

**Contents.** `do_index_node_with_plan` (`raisin-indexer/src/tantivy_engine/indexing_impl.rs:20-73`)
computes `flatten_properties` **once** from `node.properties` (`:33`; `properties.rs:23` takes only
`plan` and `properties` — no locale), writes the default-language document, then at `:46` does
`if let Some(translations) = &node.translations` and loops `supported_languages`, substituting only
`translations.get(&format!("name_{}", lang_code))` (`:54-60`) while reusing `&flattened.content`
(`:67`). So a DE document is `{language:"de", name:<German title>, content:<English body>}`.
**[verified]**

**Correction to the earlier draft.** `node.translations` (the inline map at
`raisin-models/src/nodes/node.rs:43`) is **not** write-only scaffolding. The node-write HTTP endpoint
reads a top-level `translations` object off the request body and calls `with_translations`
(`raisin-transport-http/src/handlers/repo/write.rs:117-119` →
`node_service/update_builder.rs:66-71,120-121` → `node.translations = Some(trans)`), and `save()`
goes through `NodeService::update_node` → `ctx.put_node` + `commit()`, so it emits a node event and
reindexes. Any client using that field gets the full N-document fan-out with byte-identical English
content. The claim "only one document is written at all" was wrong. **[verified]**

Drift at that same endpoint: in **commit mode** (`write.rs:80-89`) the identical input is stuffed
into `properties["translations"]` instead of the top-level field. One endpoint, two modes, two
different destinations for the same key. **[verified]**

**The per-locale loop is duplicated.** `tantivy_engine/batch.rs:93-101` is a second copy of
`indexing_impl.rs:46-60` — same `if let`, same `for lang_code in &supported_languages`, same
`name_{lang}` lookup, same shared English content. These are the two mirrored paths that will drift
under any change proposed below, and that is the risk, not merely two files to edit. **[verified]**

**Analyzer.** `schema.rs:30-39` builds one `TextOptions` with `.set_tokenizer("default")` (`:33`) and
clones it into `name` (`:38`) and `content` (`:39`); `language` is `STRING|STORED` (`:21`). The only
other `set_tokenizer` in the workspace is `"raw"` for `shape_types` (`:48`) — verified with
`grep -rn "set_tokenizer|_stemmer|tokenizers()" crates/`, which returns exactly `schema.rs:33`,
`schema.rs:48` and four lines inside `language.rs`. `register_language_tokenizer`
(`language.rs:33-45`) registers under `format!("{}_stemmer", language)` (`:35`) for 16 languages, and
**no field names it**. `git log --oneline -- .../language.rs` = one commit (`ce41ccf6`): this has
never worked. **[verified]**

Tantivy 0.25's `"default"` = `SimpleTokenizer` + `RemoveLongFilter::limit(40)` + `LowerCaser`
(`tantivy-0.25.0/src/tokenizer/tokenizer_manager.rs:59-65`); `en_stem` is registered at `:67-74` and
unused. `SimpleTokenizer` splits on `!c.is_alphanumeric()` (`simple_tokenizer.rs:34,46`), CJK is
alphanumeric, and `RemoveLongFilter`'s predicate is `token.text.len() < limit` on **UTF-8 bytes**
(`remove_long.rs:27-28,44-46`) — a 14-char CJK run is 42 bytes and is **discarded**. **[verified]**

The docs claim otherwise — "automatic stemming" (`docs/website/docs/access/sql/fulltext.md:10`),
"language-specific stemming applied" (`:57`), "the appropriate stemming algorithm" (`:67`). All three
are false against `schema.rs:33`. **[verified]**

### 2d. What a German query returns, and what an English one returns

Language is a hard `MUST` exact term (`search.rs:31-41`), and the text query is built by
`build_fuzzy_query` (`search.rs:28` for any non-wildcard query) which does
`query_parser.set_field_fuzzy(fields.name, true, 1, true)` and the same for `content`
(`search.rs:114-115`). Tantivy 0.25's signature is `set_field_fuzzy(field, prefix, distance,
transpose_cost_one)` (`query_parser.rs:306-312`), so **every query term is a prefix-fuzzy term at
edit distance 1**. **[verified]**

That materially changes the user-visible story the earlier draft told:

- **`Datenbank` → `Datenbanken` matches** (prefix, 0 edits). The reverse does not (two deletions).
  Compound splitting still never happens. **[verified/inferred]**
- **`cafe` ↔ `café` matches** (one edit). No ASCII-folding filter exists, but fuzzy covers this case.
  **[verified/inferred]**
- **CJK is unaffected and still broken** — the token is deleted at *index* time, so there is nothing
  for a fuzzy query to match. **[verified]**
- Prefix + distance-1 on **every** term is a large, undocumented recall/precision and latency
  decision. It is also why "add stemmers" will *change* result sets in both directions, not just
  improve them. **[verified/inferred]**

Concretely:

- `FULLTEXT_MATCH('datenbank', 'de')` — searches only `language:"de"` documents, which exist only if
  something populated `node.translations`; their body is English regardless. **[verified]**
- `FULLTEXT_MATCH('database performance', 'english')` — **zero rows, always.**
  `docs/website/docs/access/sql/raisinsql.md:532` documents the parameter as
  `'english' | 'german' | 'french' | 'spanish' | 'simple'`, with examples at `:548` and `:567`; the
  index stamps ISO 639-1 codes from the repo config (`fulltext/handler.rs:290-291`, written at
  `tantivy_engine/document.rs:49`), and `predicate_ops.rs:29-36` lifts the second argument as a bare
  `Literal::Text` and passes it through unchanged into the `MUST` term. `'english' != 'en'`.
  **This is the single highest-impact user-facing bug in the area.** **[verified]**
- `HYBRID_SEARCH(...)` hardcodes `"en"` at `table_function.rs:412`,
  `hybrid_search/fulltext.rs:32` (with `// TODO: Get from request or config`) and
  `mcp/services/search.rs:55` — the surface an agent calls. **[verified]**

On the semantic side, locale changes only presentation: `execute_vector_scan` retrieves locale-blind,
then per hit loops `for locale in &locales_to_use` (`vector_scan.rs:129` for the locale set, `:155`
for the loop) calling `resolve_node_for_locale` (`helpers.rs:176`) and yielding one row per locale,
each with the identical `result.distance` (`vector_scan.rs:179-182`). **[verified]**

### 2e. Can the same source document appear twice in one result set?

**Fulltext: no, by accident of the `MUST` filter.** One document per (node, language)
(`document.rs:25`), the filter admits one, `extract_results` does no node-level dedup. Widen the
filter and you get N identical-content rows. **[verified]**

**Vector: yes, twice over.**

1. `locale IN ('en','de')` yields one row per locale per hit (`vector_scan.rs:155`;
   `plan_enum.rs:31-32` documents "return one row per locale per node"), so `LIMIT k` can return
   `k × |locales|` rows. **[verified]**
2. **Chunked vector search is dead.** HNSW points are `format!("{}#{}", node_id, chunk_index)` when
   `total_chunks > 1` (`embedding/handler.rs:255-259`), and `vector_scan.rs:136` passes
   `&result.node_id` verbatim to `nodes().get(...)`, warning "Node not found" at `:188`.
   `parse_chunk_id` (`raisin-hnsw/src/types.rs:452`) and `deduplicate_by_document` (`:477`) exist but
   their only caller is `search_chunks` (`engine/search.rs:145`), which has **no production caller**
   (`grep -rn search_chunks crates/ packages/ docs/` finds only the crate README). The live SQL path
   is `search_with_threshold`. **[verified]** *(RocksDB is unaffected — see the `store_embedding`
   note in §2a.)*

---

## 3. The strategy decision

### The two problems people call "multilingual" are different

For RAG the corpus is **uploaded files**: a German PDF is a German document, not a German rendering
of an English one. Its problem is *cross-lingual retrieval over a mixed-language corpus*, solved by
the embedding model and the analyzer, not the key layout — there is one text per document, so
per-locale embeddings cost nothing and buy nothing there.

The overlay problem is the CMS content in the same repo: one node, EN base + DE overlay. That is
where the options bite. Both share one index.

### Option A — base-language-only embeddings + a multilingual model

- Storage 1.0×; no key change; zero re-embed on translation change. **[verified]**
- Returns the right locale for *display* (`vector_scan.rs:155` already resolves per locale); recall is
  whatever the model's cross-lingual alignment gives. **[verified]**
- **Premise:** requires a genuinely multilingual model. The product default is
  `text-embedding-3-small` (`raisin-embeddings/src/config.rs:152-159`); the model on this machine is
  `nomic-embed-text` at 768 dims (`provider.rs:466`), English-centric. **A green proof run on
  nomic-embed-text proves the mechanism, not cross-lingual retrieval.** **[verified]**
- **The gap the earlier draft called disqualifying:** nothing records or checks multilinguality
  (`embedder.rs:30-37`, `TenantEmbeddingConfig`). But that gap is closed by change **#14**, which the
  same draft sized as *small and non-breaking*. It is not a disqualifier; it is one small change.
  **[verified]**

### Option B — one embedding per locale, locale in the key, one HNSW partition per locale

- Storage `|supported_languages|` — 4–10×, and wasteful: overlays are sparse, so most locales would
  embed identical text. **[verified]**
- Breaking on the RocksDB key; HNSW partitioning means new signatures across the engine surface.
- Re-embed only the changed locale; locale is native to the retrieval unit.

### Option C — embed the *resolved* text per locale, dedup back to the source node

Write side as B but restricted to locales that actually have an overlay; read side collapses to one
row per source node.

**Three of C's load-bearing arguments are refuted by the code.** They are corrected here rather than
deleted, because the corrections change the recommendation.

1. **"A is already violated by the other half of the stack — fulltext writes one document per
   supported language."** Only when `node.translations` is populated (`indexing_impl.rs:46`). That
   map is written by one HTTP endpoint (§2c) and by nothing in the live translation subsystem, so on
   a repo whose translations go through `TranslationService` or `UPDATE … FOR LOCALE`, fulltext
   writes **one** document — the same count as embeddings. The asymmetry is real for one endpoint's
   clients, not universal. **[verified]**
2. **"C's read side is 'call the function that is already there'."** No.
   `search_with_threshold` (`engine/search.rs:53-113`) over-fetches `k*5` **only** when a workspace
   filter is present, applies the 0.6 threshold, then `results.truncate(k)` at `:113` *before*
   returning. Dedup downstream can only shrink k, never restore it. The 2×/10× over-fetch lives in
   `search_chunks` — a different function, different request and result types, not called by
   `vector_scan`. C needs its own over-fetch and its own dedup. **[verified]**
3. **"B's per-locale HNSW partitions are worse for memory; C keeps one index per branch."**
   Backwards. The HNSW cache is weight-bounded in **bytes** (512 MB total,
   `raisin-server/src/startup/indexing.rs:45-47`, weighed by `estimated_memory_bytes`,
   `engine/mod.rs:97-104`). Partitioning changes granularity, not total bytes. And `HnswIndex::add`
   calls `ensure_mutable` (`index.rs:204-229`), which promotes a memory-mapped `Viewed` index to
   fully `Loaded` — so C's single 8-locale index must be resident **in full** to accept one write,
   while B loads only the touched locale. **C is the higher-peak-RSS option.** **[verified]**

Two further problems with C that neither draft priced:

4. **Near-duplicate crowding.** A genuinely multilingual model maps EN and DE of the same sentence to
   nearby vectors — that is A's premise, which C endorses. Under C those are 8 separate points in one
   index, so they are each other's nearest neighbours. `search_with_threshold` truncates to k
   *before* any dedup, so a top-10 can be 8 locales of one document plus 2 others → 3 distinct
   results. **The better the cross-lingual alignment, the worse the collapse.** Under A it cannot
   happen. **[verified/inferred]**
5. **C makes `vector_scan` disagree with every other scan executor about locale semantics.**
   `plan_enum.rs:30-32` documents the contract as "one row per locale per node", and
   `prefix_scan.rs:98-107` already scales its scan limit by `get_locales_to_use(ctx).len()` to honour
   it. C's "one row per node, best locale wins" means `locale IN ('en','de')` returns two rows through
   a table scan and one through a vector scan in the same query. Either the contract changes
   everywhere — much larger than listed — or vector search becomes the one executor with different
   semantics. This is the drift rule C was argued *for*. **[verified]**

### Option D — smuggle the locale into `source_id`

**Reject.** It is the composite-string bug class CLAUDE.md names as #1; `parse_chunk_id` splits on the
last `#` so `{node}@de#3` yields a non-node source id; and node ids are not constrained to exclude
`@`. **[verified]**

### A cheaper variant of C that nobody costed

Overlays are **sparse** (`locale_overlay.rs:74-91`) and `extract_embeddable_content` is shape-driven,
so for many nodes the locale-resolved text of the *embeddable* fields is byte-identical to the base.
`EmbeddingData` already carries `text_hash = hash_text(chunk_content)` (`embedding/handler.rs:271`).
**Hash the locale-resolved text and skip the embed and the HNSW point when it equals the base hash.**
That collapses the multiplier from "locales with any overlay" to "locales that actually change
embeddable text" — cheaper than C on spend, on memory and on crowding, for a few lines against a
field that already exists. **[verified — the field exists; the saving is inferred]**

### The pick: **not C, not yet**

Ship **Phase 0 + Phase 1 + #14 + #18**, then **measure**, then decide C vs A.

Reasons, against this codebase:

1. **The motivating corpus gets nothing from per-locale embeddings.** Uploaded RAG files carry no
   overlays, so C's multiplier there is 1.0× — behaviourally *identical to A*. Everything C buys is
   for translated CMS content, outside the stated scope. Yet C carries all the breaking changes and
   all the migration risk. Even at 1.0× it is not free: "locales with an overlay for this node" needs
   a `cf::TRANSLATION_DATA` prefix probe per node per embed job — 50k extra RocksDB scans on a corpus
   where the answer is always "none". **[verified/inferred]**
2. **A's disqualifier costs one small non-breaking change** (#14), versus C's breaking storage-key
   migration, HNSW id format change, new translation event system, staleness backfill, and a rebuild
   job that does not exist. The earlier comparison was not apples to apples. **[verified]**
3. **The cross-lingual premise is cheap to test and has not been tested.** See "The gate" below.

Adopt A's one good idea regardless: **a multilingual model is the right default**, because it is what
makes a French query find a French-only uploaded document with no English counterpart — something no
per-locale key layout can do.

### Hard prerequisites the earlier draft omitted

- **HNSW dimensionality is hardcoded process-wide.** `startup/indexing.rs:47` constructs
  `HnswIndexingEngine::new(hnsw_path, cache_size, 1536)` — one width for every tenant.
  `HnswIndex::add` hard-errors on `vector.len() != dimensions` (`index.rs:265-271`) and the handler
  propagates it (`embedding/handler.rs:308`). Every candidate multilingual model is a different
  width: multilingual-e5-base and LaBSE 768, e5-large 1024, text-embedding-3-large 3072. A tenant
  switching gets the provider call **paid**, the vector **written** to `cf::EMBEDDINGS`, then the
  HNSW add rejected and the job failed — **and retried, paying again**. The trap already exists here:
  config default is 1536 (`config.rs:154`) while `nomic-embed-text` returns 768 (`provider.rs:466`).
  **Per-tenant HNSW dimension is a hard prerequisite for the "multilingual model" advice.**
  **[verified]**
- **No cluster lease on embedding jobs.** The replication apply path publishes node events
  (`application/node_operations/create_node.rs:161`, `set_property.rs:107`, `move_rename.rs:249`,
  `crdt_ops.rs:416,476`, `legacy_node_ops.rs:277,454`), but they are filtered at
  `node_handlers.rs:139` — so today replicas do **not** double-embed. Land #9 (translation events)
  and #18 (apply translations on replicas) and each replica starts generating locally from its own
  events; job dedup is per-**process** (CLAUDE.md), and a grep of `jobs/handlers/embedding/` and
  `node_handlers.rs` for `lock|lease|try_acquire` returns **zero**. On a 3-node cluster with 8
  locales that is up to **24× provider spend**. A per-(node, locale) `raisin_locks` lease belongs in
  the plan. **[verified]**
- **No compaction filter on `cf::EMBEDDINGS`.** The revision is the **last** key segment
  (`storage.rs:71-72`) and `store_embedding` is a bare `put_cf` (`:246`); the only registered filter
  factory is spatial (`raisin-rocksdb/src/lib.rs:437`). Every revision leaves a full vector behind
  forever — the exact trap CLAUDE.md documents for `cf::SPATIAL_INDEX` (OPEN-ITEMS §2.99). Today only
  base-node writes mint revisions; under #9, a translator editing one node 20 times across 8 locales
  leaves 160 dead vectors (~1 MB) for that node. This is **independent and permanent**, not something
  #10 fixes. **[verified]**

### The cost numbers

Inputs, all verified: 1536 dims hardcoded (`startup/indexing.rs:47`); `QuantizationType::F32` default
(`raisin-hnsw/src/types.rs:105-118`) = **6,144 B/vector**; `HnswParams` connectivity/expansion all 0
so usearch defaults (M=16) apply; global cache **512 MB** (`startup/indexing.rs:45-47`) weighed by
`estimated_memory_bytes` = usearch bytes + `len*64` + `len*80` (`index.rs:374-382`).

| scenario | points | vectors | graph | maps | total |
|---|---|---|---|---|---|
| 50k docs, 1 locale | 50,000 | 307 MB | ~14 MB | ~7 MB | **~328 MB** |
| 50k docs, 8 locales (Option C, one index/branch) | 400,000 | 2.46 GB | ~112 MB | ~58 MB | **~2.63 GB** |

**[verified inputs; arithmetic inferred]**

One tenant at 50k documents and one locale already consumes **65% of the entire global HNSW cache**.
Under C, a single moka entry of ~2.63 GB against a `max_capacity` of 512 MB **can never be retained**:
every query becomes a cache miss plus a full sidecar reload, and every write promotes the whole thing
to `Loaded`. Against CLAUDE.md's memory history (19 GB RSS incident; 9.3 GB allocator-retention
investigation; jemalloc not serving RocksDB's C++ heap), this is a re-run of a known outage class.
**Cache budget and dimension must become configuration before any per-locale plan.**

**Sidecar write amplification.** `raisin-hnsw/src/persistence.rs:48-63` serializes `node_to_key` **and**
`key_to_meta` in full to JSON on every save, and the snapshot task runs every 60s for any dirty index.
At 50k points that sidecar is ~9 MB; at 400k it is ~70–75 MB **rewritten every 60 seconds** for as
long as a backfill runs. There is no incremental format. Multiply by tenants, on a box CLAUDE.md
documents as disk-constrained. **[verified structure; sizes inferred]**

**Read cost that gets worse before it gets better.** `get_embedding`'s v2 path falls through to a
`prefix_iterator_cf` over the **whole workspace** filtered by `source_id` in a loop
(`storage.rs:306-334`), because `embedder_hash` sits before `source_id` in the key. Every per-node
embedding lookup already walks every embedding in the workspace; multiplying rows by locale count
makes that walk 8× longer. Moving the locale segment "after workspace_id, before embedder_hash" is the
right placement but does not fix the pre-existing full scan. **[verified]**

### The gate: measure before deciding C vs A

The cross-lingual premise should gate this decision, not be argued around. Minimum harness: ~200
documents in EN/DE/JA, ~50 queries per language with known relevant documents; measure recall@10 and
MRR for (a) query language == document language and (b) query language != document language, per
candidate model. Decision rule:

- Cross-language recall@10 within a few points of same-language → **A + #14 suffices; Phase 2 is
  unnecessary.**
- Cross-language recall collapses → **C is justified**, and the same measurement yields the
  over-fetch factor the dedup needs (which must be derived from the locale count, not the 2×/10×
  constants in `search_chunks` that were calibrated for chunks).

Nothing resembling such a harness exists in the tree. **[verified]** Without it the failure mode is
exactly the one this document warns about: silently poor recall, per tenant, with no error anywhere.

---

## 4. Fulltext is a separate decision, and the answer is different

Tokenizer per language is **a correctness bug, not a quality issue**: a Japanese document indexed
through `RemoveLongFilter(40 bytes)` has its tokens *deleted*, so the query returns zero rows with no
error — silent data loss, not a ranking regression. **[verified]**

### Today

- `name` and `content`: `set_tokenizer("default")` (`schema.rs:33`, cloned `:38`, `:39`).
- `"default"` = `SimpleTokenizer` + `RemoveLongFilter(40)` + `LowerCaser`. No stemmer, no folding, no
  CJK segmentation.
- 16 `{lang}_stemmer` analyzers registered (set listed at `language.rs:11-27`, registration `:33-45`) and
  referenced by **nothing**.
- One index directory per (tenant, repo, branch): `index_manager.rs:61` (cache key), `:70` (path).
- Query side is prefix-fuzzy distance 1 (`search.rs:114-115`).

### Recommended shape

**One Tantivy index per (tenant, repo, branch, language), with that language's analyzer baked into the
schema.** `make_key`/`get_index_path` (`index_manager.rs:61,70`) gain a segment; `build_schema()`
(`schema.rs:15`) gains a `language: &str`; `language.rs:33-45` gets wired to `name` and `content` and
gains `AsciiFoldingFilter`; `indexing_impl.rs:35-73` **and its duplicate at `batch.rs:78-121`** route
each locale document to its own index — **unify those two into one helper in the same change, or this
becomes the next mirrored-path bug.** The `language` `MUST` term becomes redundant but harmless.

| language class | analyzer |
|---|---|
| European (`en, de, fr, es, it, pt, ru, nl, da, fi, hu, no, ro, sv, tr, ar` — the 16 in the `stemmer_for` match, `language.rs:11-27`) | `SimpleTokenizer` → `RemoveLongFilter(≥60)` → `LowerCaser` → **`AsciiFoldingFilter`** → **`Stemmer(lang)`** |
| ja / zh / ko | a real segmenter — `lindera` (ja/ko/zh) or `cang-jie`/`jieba` (zh). `SimpleTokenizer` cannot segment these and `RemoveLongFilter(40)` deletes what it produces. Raise the limit regardless. |
| unknown / `simple` | today's `"default"`, explicitly named as the fallback |

### The migration is real work, not a `SCHEMA_VERSION` bump

The earlier draft's "bump `SCHEMA_VERSION` and the sidecar rebuilds it" is **wrong on production
tenants**, twice over: **[verified]**

- `maybe_dev_auto_rebuild` (`jobs/handlers/fulltext/handler.rs:82-92`) returns immediately unless
  `context.tenant_id == "default"`, and its own doc comment says production "is left to an explicit
  operator rebuild — a stale index keeps working (degraded) until then." On a stale index
  `index_manager.rs:77-90` logs `tracing::warn!` and then opens it anyway.
- Worse after the path changes: `is_index_stale` (`index_manager.rs:149-158`) probes
  `base/tenant/repo/branch/meta.json` and returns `false` when absent ("not-yet-created"). Once the
  layout gains a language segment, the probe finds nothing at the new path, staleness is **never
  reported**, `SCHEMA_VERSION` is inert for this migration, and the old per-branch directories are
  orphaned with nothing noticing. Every `FULLTEXT_MATCH` returns zero rows until an operator runs a
  manual rebuild, per tenant.

**A written per-tenant migration/rebuild job is part of Phase 1, not a footnote.**

Two more, cheaply:

- **Registration must move into index open.** Tantivy resolves a field's tokenizer by name at **both**
  write and read. `register_language_tokenizer` is called only from write paths
  (`indexing_impl.rs:30,52`; `batch.rs:44,46`); `get_or_create_index` never registers and neither does
  the search path (`search.rs:112` just resolves what the schema names). Bake `de_stemmer` into the
  schema and any process that opens an index **search-first** hits an unregistered tokenizer.
  Registration belongs in `get_or_create_index`. **[verified]**
- **Per-language indices inherit the file-handle argument that was used against Option B.** Tantivy
  indices are directories of many segment files; the cache weigher is a flat 30 MB-per-entry fiction
  (`index_manager.rs:38`), so the 512 MB budget is really a count of ~17 indices regardless of true
  size; and branch fork **copies** the index (`fulltext/handler.rs:208-230`, `do_branch_created`). So
  8 languages × every branch. The argument cannot both sink B and spare this — it is accepted here
  because for fulltext there is no alternative (analyzers bind to schema fields), not because it is
  free. **[verified]**

Also fulltext-only, independent of analyzers:

- **Normalize the language argument at the SQL boundary** (`predicate_ops.rs:29-36`), or fix the docs
  (`raisinsql.md:532,548,567`) — but not neither.
- **Stop hardcoding `"en"`** in `table_function.rs:412`, `hybrid_search/fulltext.rs:32`,
  `mcp/services/search.rs:55`.

### Why fulltext's answer differs

An embedding model can partially cover a locale you failed to index. A German stemmer cannot be
approximated by an English one; a CJK segmenter cannot be approximated by whitespace. **Per-language
analyzers are mandatory in a way per-locale vectors are not.**

---

## 5. The divergence risk

CLAUDE.md names mirrored-path drift as this repo's #1 bug class. The two halves of search are that,
at the top level, and there are now **six** shipped divergences: **[verified]**

1. **Remote events.** Fulltext indexes replicated nodes (`node_handlers.rs:71-72`); embeddings skip
   them (`:139-147`) with a comment claiming they are "replicated separately" — they are not
   (no `OpType` carries a vector). Replicas have fulltext and no vectors.
2. **`FullTextScan` hardcodes `"en"`** into `node_to_row` (`physical_plan/fulltext.rs:144`); the file
   never calls `resolve_node_for_locale` (only `use super::scan_executors::node_to_row` at `:8`).
   Every other scan executor passes the query locale on a resolved node — `vector_scan.rs:155-165`,
   `prefix_scan.rs:124`, `property_index_scan.rs:86`, `compound_scan.rs:163`, `spatial_scan.rs:165,301`,
   `reference_scan.rs:133`, `table_scan.rs:101,218`, `property_range_scan.rs:178`. And the locale
   predicate really is consumed and dropped: `extract_locale_predicate`
   (`raisin-sql/src/analyzer/semantic/predicates.rs:213-304`) returns `(vec![locale], None)`.
   **`FULLTEXT_MATCH(...) AND locale='de'` silently returns untranslated rows.**
3. **Two unrelated locale mechanisms with the same name.** `FULLTEXT_MATCH`'s second argument is a
   Tantivy facet; `WHERE locale='de'` is the overlay resolver. They never meet.
4. **Locale normalization is asymmetric.** `analyze_translate` validates `is_alphanumeric() || '-' ||
   '_'` (`statement_analysis.rs:254-260`) and stores `stmt.locale` **verbatim** (`:312`), while reads
   go through `LocaleCode::parse` (`helpers.rs:176`), which lowercases the language and **uppercases
   the second part** (`locale_code.rs:129-147`). `FOR LOCALE 'de-ch'` writes `de-ch`;
   `WHERE locale='de-CH'` reads `de-CH`. Zero rows, no error.
5. **`LocaleCode::parse` uppercases scripts too.** `is_valid_region` accepts a 4-letter script code
   (`locale_code.rs:165-169`), but `:131` has already run `to_uppercase()` — so `zh-Hans` normalizes to
   **`zh-HANS`**, not the BCP-47 form any external caller sends. And 3-part locales are rejected
   outright (`:148-151`), so `zh-Hans-CN` — documented as valid at
   `raisin-models/src/translations/mod.rs:67` — cannot parse at all.
6. **The per-locale fulltext loop exists twice** (`indexing_impl.rs:46-60` and `batch.rs:93-101`).

**Which write paths actually reach the embedding gate — three different behaviours:** **[verified]**

| path | emits node event? | embeds? |
|---|---|---|
| transaction `put_node`/`add_node`, SQL DML, WS create | yes (via `commit → emit_node_events`) | **yes** |
| replication apply (`node_operations/event_helpers.rs:41`, called from `create_node.rs:161`, `set_property.rs:107`, `move_rename.rs:249`, `crdt_ops.rs:416,476`, `legacy_node_ops.rs:277,454`) | yes | **no** — filtered at `node_handlers.rs:139` |
| repository layer (`nodes/crud/create/add.rs`, `crud/update.rs`, `queries/property.rs`) | **no** — no `event_bus`/`publish` token in any of them | **no**, silently |

A write through `storage.nodes().update(...)` re-embeds and re-indexes **nothing**. That is
CLAUDE.md's own documented rule, and it is a third behaviour nobody enumerated.

**The rule to adopt, and to put in CLAUDE.md:** whatever locale set the embedding path indexes, the
fulltext path must index the same set, from the same resolved text, decided in **one** place — a
single `locales_to_index(node, repo_config) -> Vec<LocaleCode>` called by both handlers.

---

## 6. Required changes, ordered

"Breaking" means a storage key layout or an index format.

### Phase 0 — bugs to fix regardless of strategy. None breaking. **Ship these.**

1. **Strip the composite id before the node fetch in vector scan.** `vector_scan.rs:136` passes
   `{node}#{chunk}` straight to `nodes().get()`. Route through `parse_chunk_id`
   (`raisin-hnsw/src/types.rs:452`) and fold with `deduplicate_by_document` (`:477`). **Two other
   consumers must use the same parser**: `hybrid_search/vector.rs:87,105` feeds `node_id` verbatim to
   `nodes().get`, and `hybrid_search/rrf.rs:81,103` **fuses the two legs on that raw id** — RRF can
   never match `doc#0` against fulltext's plain `doc`, so hybrid degrades to an unfused union with no
   error. Note `search_with_threshold` truncates to k at `engine/search.rs:113` *before* any dedup, so
   dedup alone shrinks results; it needs its own over-fetch.
   *Small–medium. Not breaking. Fixes chunked vector search and chunked hybrid search.*
2. **Fix the byte-slicing panics on non-ASCII text.** `embedding/handler.rs:156-161` does `&text[..200]`
   guarded only by `text.len() > 200` — a hard panic, so the chunking fallback at `:207-214` does not
   catch it. `eval/async_eval.rs:236,246,264-268` does `&text[..text.len().min(50)]` on the **query**
   path, the last at INFO. The correct idiom is in the same file at `handler.rs:269`
   (`.chars().take(200).collect()`).
   *Small. Not breaking. Blocks any Japanese/Thai/Arabic content or query.*
3. **Fix the locale hardcodes.** `physical_plan/fulltext.rs:144` (plus a `resolve_node_for_locale`
   call), `table_function.rs:412`, `hybrid_search/fulltext.rs:32`, `mcp/services/search.rs:55`.
   *Small–medium. Not breaking.*
4. **Fix `LocaleCode::parse` before normalizing the SQL write path.** Routing `stmt.locale` through
   the current parser does not merely orphan mis-cased overlays — it **hard-rejects** locales the
   char-class check accepts today: `FOR LOCALE 'zh-Hans-CN'` becomes a parse error
   (`locale_code.rs:148-151`), and `zh-Hans` normalizes to `zh-HANS` (`:131`). Fix the parser (preserve
   script casing, accept 3-part chains) **first**, then normalize `statement_analysis.rs:254-260`.
   *Small each. Breaking in effect — pre-existing non-normalized overlays are at keys no read path
   finds; needs a `cf::TRANSLATION_DATA` locale-segment rewrite scan, or an explicit "orphaned" note.*
5. **Accept the documented `FULLTEXT_MATCH` spellings, or fix the docs.** `predicate_ops.rs:29-36`;
   `raisinsql.md:532,548,567`.
   *Small. Not breaking. Highest user-facing impact in the area.*

### Phase 1 — fulltext. Independent of the embedding decision, highest value per unit of work.

6. **One Tantivy index per (tenant, repo, branch, language), analyzer baked in.**
   `index_manager.rs:61,70`; `schema.rs:15`; `language.rs:33-45` (wire up + `AsciiFoldingFilter`);
   **unify `indexing_impl.rs:35-73` with `batch.rs:78-121` into one helper**; move
   `register_language_tokenizer` into `get_or_create_index`.
   *Large. **Breaking — index format and on-disk layout.** Migration is **not** a `SCHEMA_VERSION`
   bump: `maybe_dev_auto_rebuild` is tenant-`default`-only (`fulltext/handler.rs:82-92`) and
   `is_index_stale` cannot see the old path once the layout changes (`index_manager.rs:149-158`). A
   per-tenant rebuild job plus deletion of the old per-branch directories must be written. No node
   data is touched — the index is fully derivable.*
7. **Add a CJK segmenter** (`lindera` / `cang-jie`) for ja/zh/ko and raise `RemoveLongFilter` above 40
   bytes for those fields. `language.rs`.
   *Medium (new dependency; check license and binary size — CLAUDE.md's disk section). Not breaking
   beyond #6.*
8. **Make fulltext read the live overlay, not `node.translations`.** `indexing_impl.rs:46-55` (and its
   `batch.rs` twin) read the legacy inline map, which only one HTTP endpoint writes
   (`repo/write.rs:117-119`). Resolution must happen on the **async** side, after `drop(tx)` at
   `fulltext/batch.rs:162`, next to `resolve_index_plan` (`:166-182`) — the write side runs under
   `spawn_blocking` (`batch.rs:200-204`, `handler.rs:327-331`) and `batch.rs:153-162` records that a
   storage read inside the open transaction reliably deadlocks.
   *Medium. Not breaking.*
14. **Record model multilinguality on `EmbedderId` / `TenantEmbeddingConfig` and warn** when a repo has
    more than one supported language on an English-centric model. `raisin-ai/src/config/embedder.rs`,
    `raisin-embeddings/src/config.rs:152-159`. Do **not** add it to `to_key_hash`
    (`embedder.rs:50-64`) — that hash is a key segment. *(Promoted out of Phase 2: this is the change
    that closes A's only gap.)*
    *Small. Not breaking, provided the flag stays out of the hash.*
18. **Apply `SetTranslation` / `DeleteTranslation` on the receiving replica.** Captured
    (`repositories/translations/replication.rs:28,43` → `operation_capture/node_ops.rs:349,381`),
    shipped (`raisin-replication/src/operation/op_type.rs:99-112`), dropped by the catch-all
    `_ => { debug!("Operation type not handled by applicator"); Ok(()) }` under the
    `// ========== Not Yet Implemented ==========` banner (`application/applicator/mod.rs:603-607`).
    The two-node test asserts only the peer's **oplog** (`two_node_replication_test.rs:742-755`).
    Today translations reach a peer only via checkpoint/SST ingest and fork copy.
    *Medium. Not breaking. Checkpoint SST ingest emits no events, so any derived per-locale index must
    also register with `derived_cache_registry` — CLAUDE.md's rule. **Prerequisite: the per-cluster
    lease below**, since applying translations on replicas is what starts N× embedding.*

### Phase 1.5 — prerequisites for *any* per-locale work. Do these before Phase 2 is even costed.

19. **Make HNSW dimensionality per-tenant.** `startup/indexing.rs:47` hardcodes 1536 process-wide;
    `index.rs:265-271` hard-errors on mismatch after the provider call is paid and the vector is
    already written to `cf::EMBEDDINGS`, and the job retries. Blocks the multilingual-model advice
    outright. *Medium. Not breaking on disk.*
20. **Add a compaction filter on `cf::EMBEDDINGS`.** Revisions are in the key (`storage.rs:71-72`),
    `put_cf` never overwrites, and only spatial has a filter (`raisin-rocksdb/src/lib.rs:437`).
    Independent of and permanent without #10. *Medium.*
21. **Take a per-(node, locale) `raisin_locks` lease around embedding generation.** Job dedup is
    per-process; zero lock/lease tokens exist in `jobs/handlers/embedding/` or `node_handlers.rs`.
    Without it, #9 + #18 on a 3-node × 8-locale install is up to 24× provider spend. *Medium.*
22. **Make the HNSW cache budget configurable**, and reconsider the flat 30 MB Tantivy weigher
    (`index_manager.rs:38`). 512 MB is a hard constant (`startup/indexing.rs:45`) and one 50k-document
    tenant already takes 65% of it. *Small.*

### Phase 2 — per-locale embeddings. **Gated on the measurement in §3.** Do not start before it.

9. **Emit an event on translation write.** Prerequisite for everything below. Both write paths:
   `translation_service/operations.rs` (bypasses `NodeService` entirely) and the transaction path
   (`commit/events.rs:103-110` passes only `changed_nodes`). Do it **once**, in a shared helper. A
   cheaper first step: fan out from the `NodeChangeInfo { translation_locale }` entries already built
   at `commit/extract.rs:73-79`.
   *Medium. Not breaking. Must land with #10, #20 and #21 or it is pure waste — worse, since each
   write mints a revision and vectors accumulate.*
10. **Locale-resolve the embedding job's text.** `embedding/handler.rs:124-138` must resolve through
    `TranslationResolver` per target locale before `extract_embeddable_content`. Locale set from the
    **shared** `locales_to_index` helper from #8. **Add the `text_hash` short-circuit**: skip the embed
    and the HNSW point when the locale-resolved text hashes equal to the base
    (`handler.rs:271` already computes `text_hash`).
    *Medium. Not breaking on its own.*
11. **Add a `locale` segment to the embedding key (v3).** `storage.rs:42-75`, after `workspace_id` and
    before `embedder_hash`.
    *Medium. **Breaking — storage key format.** Neither migration option in the earlier draft works:*
    - *(a) "keep the v2 reader as a fallback" is **unsound as written**. The dispatcher is
      count-based on a NUL split (`storage.rs:101-124`: `String::from_utf8_lossy(key).split('\0')`,
      `parts.len() >= 9` vs `>= 6`). A v3 key has 10 parts — but so does any v2 key whose trailing
      16-byte descending HLC contains a `0x00`, which it can: `encode_descending` is `!timestamp_ms`
      then `!counter` (`raisin-hlc/src/lib.rs:132-144`), so a `0x00` appears wherever the raw
      millisecond timestamp has a `0xFF` byte. This is verbatim the trap CLAUDE.md documents for
      `ORDERED_CHILDREN`. **A version byte, not a segment count, is the only safe discriminator** —
      and adding one is itself a format change.*
    - *(b) "drop `cf::EMBEDDINGS` and re-run the existing rebuild endpoint" **cannot work**.
      `management/database/vector_embeddings.rs:216` enumerates
      `list_embeddings(&tenant,&repo,&branch,"staff")` — **existing embeddings, not nodes**. With the
      CF dropped the list is empty and `:230-241` takes the `total_embeddings == 0` branch, sets
      `{queued:0}` and calls `mark_completed`: a **silent success that re-embeds nothing**. It is also
      workspace-hardcoded to `"staff"` (`:216,:247,:270`) and gated on
      `force || vector.len() != expected_dims` (`:248`) — a dimension-repair tool, not a migration
      tool. **A real backfill job must be written.***
    *Note: `get_embedding`'s v2 path is already an O(all vectors in the workspace) scan
    (`storage.rs:306-334`); the locale multiplier makes it 8× worse before the new prefix helps.*
12. **Carry the locale in the HNSW point id and fold it back.** `{node_id}@{locale}#{chunk}` at
    `handler.rs:255-259`, parsed by a **single** shared `parse_point_id` next to `parse_chunk_id`
    (`types.rs:452`), used by `vector_scan.rs:136`, `hybrid_search/vector.rs:87` **and**
    `hybrid_search/rrf.rs:81,103` — one parser everywhere, or this is the next mirrored-path bug.
    *Medium. Index format, HNSW only — rebuild, no migration. Must land atomically with #1. Also
    requires deciding the `plan_enum.rs:30-32` "one row per locale per node" contract question in §3.5.*
13. **Wire `translation_staleness` as the re-embed signal, and close its holes first.** It is
    **pull-only** (only consumers are two HTTP `raisin:cmd` commands routed at `repo/commands.rs:160-171`
    into `repo_command/translations.rs:262,329`) and blind to SQL: `store_hash_record` writers are only
    `translation_staleness/mod.rs:367,423` and `translation_service/operations.rs:141` — the 370-line
    `dml_executor/translate.rs` has none, so a translation written through `psql` is permanently
    `unknown_fields` and can never report stale.
    *Medium. Not breaking.*

### Phase 3 — hygiene, cheap, worth doing while in the area

15. **`store_extracted_text` is dead config.** Set at `event_handler/asset_processing.rs:179,211`,
    declared at `raisin-ai/src/rules/settings.rs:65`, read by nothing. Extracted PDF text is only
    returned as job-result JSON and never written to node properties — so **whatever puts uploaded-file
    text into a node is not this handler**, and I could not find what does. Worth confirming before
    designing chunk-node layout, because that is where a per-document `language` would be stamped.
16. **`trigger_embedding` is dead config too** — and it is the stronger finding, because it is the
    asset pipeline's own "trigger embedding generation after text extraction" switch. Set at
    `event_handler/asset_processing.rs:180,212`, declared at `raisin-ai/src/rules/settings.rs:61`,
    merged at `:143`, and **read by nothing**. This is the direct cause of the RAG-path question this
    document opens with. **[verified]**
17. **Image embeddings are computed and thrown away.** `asset_processing/handler.rs:345-346` gates on
    `generate_image_embedding`, `:417-421` puts the CLIP vector into `result.image_embedding`, and that
    is the end of it — the vector is only serialized into the job-result JSON.
    `grep -rn 'EmbeddingKind::Image' crates/` matches **only** `raisin-ai/src/config/embedder.rs:83,91`
    (the to/from key-char arms); nothing ever calls `store_embedding` with kind `Image`, and the sole
    construction site is `EmbeddingKind::Text` (`embedding/handler.rs:265`). The `'I'` kind byte in the
    key format is unreachable dead space and **image search cannot work at all**. **[verified]**
23. **`SplitterType` is dead config** — `raisin-ai/src/config/chunking.rs:21-23`; `grep -rn '\.splitter'
    crates/` returns nothing. `TextChunker::chunk_text` (`chunking/mod.rs:94-98`) branches solely on
    `tokenizer_id`, whose default is `None` (`chunking.rs:41`), so `chunk_size` 256 is **characters**,
    not the documented tokens (`:11-14`), and the token estimate is `content.len() / 4` — **bytes**
    (`mod.rs:119`).
24. **`chunk_text` panics on multibyte text.** `chunking/mod.rs:106` slices `text[current_offset..]`
    where `current_offset = start_offset + 1` (`:112`) — a byte index, never a char boundary in a
    multibyte script. Safe on the first chunk, deterministic from the second. Not reachable on defaults
    (`chunking: None`, `raisin-embeddings/src/config.rs:159`) — which is exactly why the first tenant to
    enable chunking on a German corpus will find it.

---

## 7. Open questions — product decisions only the owner can make

1. **Should a French user searching in French find an English-only document?** Yes → cross-lingual
   retrieval is a hard product requirement and #19 (per-tenant dimensions) is on the critical path. No
   → retrieval is locale-scoped and a French query must return nothing.
2. **When a node matches in two locales, one row or two — and which locale?** Today it is two or more
   rows with identical distances. Note this is not purely a preference: "one row" makes `vector_scan`
   disagree with every other scan executor's documented contract (`plan_enum.rs:30-32`,
   `prefix_scan.rs:98-107`).
3. **Is `default_language` at repository level the right granularity for RAG?** It is documented
   immutable (`raisin-context/src/repository/config.rs:18-20`) yet one repo holds a mixed-language
   uploaded corpus. Does an uploaded file need its own `language` property?
4. **For an uploaded file of unknown language: detect, ask, or assume the repo default?** No
   language-detection code exists anywhere in the workspace.
5. **Should stale translations be retrievable?** When the base changes, the overlay is stale by hash
   (`raisin-models/src/translations/hash_record.rs:169-171`) and its embedding with it. Serve the stale
   translation, fall back to the fresh base, or suppress?
