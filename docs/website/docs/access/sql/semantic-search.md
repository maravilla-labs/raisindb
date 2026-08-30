---
title: Semantic & hybrid search
description: Three SQL entry points over one search engine — lexical, vector, and rank-fused — with an explicit workspace scope, per-tenant embedders, and the operational commands that keep the index honest.
---

# Semantic & hybrid search

RaisinDB searches text two ways at once. The **lexical** leg is Tantivy: tokens,
stems, typo tolerance. The **vector** leg is an HNSW index over embeddings: meaning,
across languages, with no shared words required. Three table functions expose them:

| Function | Legs | Use it for |
|---|---|---|
| `FULLTEXT_SEARCH(query, language, …)` | lexical only | exact words, names, codes, quoted phrases |
| `KNN(query, limit, …)` | vector only | meaning; cross-lingual; "more like this" |
| `HYBRID_SEARCH(query, limit, …)` | both, rank-fused | the default for RAG and site search |

All three return the **same columns**, apply the **same row-level security**, and take
the **same workspace scope**. `KNN` is `HYBRID_SEARCH` with the lexical leg switched
off; `FULLTEXT_SEARCH` is the same with the vector leg off.

Every example on this page was run against a live server with a local
`bge-m3` embedder. Output is verbatim.

## The workspace scope is required

The universe a search covers is the single most consequential thing about it, so it
is written in the query and cannot be defaulted:

```sql
SELECT path, workspace_id, score, fulltext_rank, vector_rank, vector_distance
FROM HYBRID_SEARCH('worn friction material on a disc', 5, workspaces => 'library');
```

```text
path          workspace_id  score    fulltext_rank  vector_rank  vector_distance
/brake-pads   library       0.03278  1              1            0.3488
/mortgage     library       0.01612  2              NULL         NULL
/espresso     library       0.01587  3              NULL         NULL
/winter-layup library       0.01562  4              NULL         NULL
```

Four spellings, and only four:

```sql
workspaces => 'library'                    -- one workspace
workspaces => 'library, handbook'          -- a list; every name must resolve
workspaces => 'content-*'                  -- a glob; a name that matches nothing is fine
workspaces => 'ALL READABLE'               -- every workspace this caller may read
```

A **name** is an assertion and a **glob** is a question, so they fail differently:

```sql
SELECT * FROM HYBRID_SEARCH('brake pads', 5, workspaces => 'nope');
-- ERROR: workspace 'nope' is not available to this query.
```

```sql
SELECT path, workspace_id FROM HYBRID_SEARCH('winter', 5, workspaces => 'content-*');
-- /post-winter  content-blog
```

`'*'` and `'ALL'` are rejected on purpose. `'ALL READABLE'` is two uppercase words that
appear in no other context, so "which of our queries go repo-wide?" is one grep:

```sql
SELECT * FROM HYBRID_SEARCH('brake pads', 5, workspaces => '*');
-- ERROR: workspaces => '*' is not a scope. Use 'ALL READABLE' for every workspace
--        you may read, or a glob such as 'content-*'.
```

Omitting the scope is an error, not a repo-wide search:

```sql
SELECT * FROM HYBRID_SEARCH('brake pads', 5);
-- ERROR: HYBRID_SEARCH requires an explicit workspace scope. Add
--        workspaces => '<workspace>' to search one, 'a, b, c' for several,
--        'content-*' for a family, or workspaces => 'ALL READABLE' for every
--        workspace you may read.
```

`ALL READABLE` really does mean *all* — including workspaces the platform created:

```sql
SELECT path, workspace_id, fulltext_rank, vector_rank
FROM HYBRID_SEARCH('expenses and receipts', 5, workspaces => 'ALL READABLE');
```

```text
path                    workspace_id  fulltext_rank  vector_rank
/expenses               handbook      1              1
/mortgage               library       NULL           2
/ms-graph-adapter       packages      2              NULL
/onboarding             handbook      NULL           3
/google-drive-adapter   packages      3              NULL
```

Name the workspaces when the answer must come from content, not from build artifacts.

### Arguments

```text
HYBRID_SEARCH  ( query [, limit] [, workspace] [, named …] )
FULLTEXT_SEARCH( query ,  language            [, named …] )
KNN            ( query [, limit]              [, named …] )
```

| Named argument | `HYBRID_SEARCH` | `FULLTEXT_SEARCH` | `KNN` | Default |
|---|:-:|:-:|:-:|---|
| `workspaces` | ✅ | ✅ | ✅ | **required** |
| `limit` | ✅ | ✅ | ✅ | 10 (100 for `FULLTEXT_SEARCH`) |
| `language` | ✅ | ✅ (also positional #2) | — | the repo's default language |
| `vector_weight` | ✅ | — | — | 1.0 |
| `fulltext_weight` | ✅ | — | — | 1.0 |
| `max_distance` | ✅ | — | ✅ | 0.6 |
| `kind` | ✅ | — | ✅ | `'text'` |

Positionals must precede named arguments, and each value may be given once. Unknown
names are rejected rather than ignored, which is what stops a silently-dead argument:

```sql
SELECT path FROM KNN('tarpaulin', 5, workspaces => 'library', language => 'en');
-- ERROR: unknown argument 'language' for KNN. Valid: workspaces, limit, max_distance, kind.
```

The `language` is an **ISO 639-1 code**. This is checked, because the index stores
two-letter codes and anything else matched zero rows forever:

```sql
SELECT path FROM FULLTEXT_SEARCH('tarpaulin', 'english', workspaces => 'library');
-- ERROR: FULLTEXT_SEARCH: language must be an ISO 639-1 code. Use 'en', not
--        'english'; the index stores two-letter codes, so anything else matches
--        no documents.
```

## Result columns

Every entry point emits the same row:

| Column | Meaning |
|---|---|
| `node_id`, `workspace_id` | the hit's identity — a node id is unique only *within* its workspace |
| `name`, `path`, `node_type` | from the node |
| `score` | the fused rank score (see below) |
| `fulltext_rank` | 1-based rank in the lexical leg, `NULL` if it did not match |
| `vector_rank` | 1-based rank in the vector leg, `NULL` if it did not match |
| `vector_distance` | cosine distance of the vector hit, `NULL` when `vector_rank` is |
| `chunk_index` | which chunk of a long document answered; `0` for an unchunked one |
| `embedding_kind` | `'text'` or `'image'` — which embedding space produced the vector hit |
| `revision`, `created_at`, `updated_at` | from the node |
| `properties` | the node's properties, already field-filtered by the permission that granted access |

They behave like ordinary columns — project them, filter on them, order by them:

```sql
SELECT path, score
FROM HYBRID_SEARCH('winter', 10, workspaces => 'ALL READABLE')
ORDER BY score DESC LIMIT 3;
```

```text
/winter-layup   0.03278688524590164
/post-winter    0.03200204813108039
/handbook-long  0.016129032258064516
```

```sql
SELECT path, node_type, score
FROM HYBRID_SEARCH('winter', 10, workspaces => 'ALL READABLE')
WHERE workspace_id = 'library';
-- /winter-layup   kb:Doc  0.0327868…
-- /handbook-long  kb:Doc  0.0161290…
-- /mortgage       kb:Doc  0.0153846…
```

A residual `WHERE` is applied *after* fusion, so `limit` means rows delivered.

## Fusion is rank-based, not score-based

Legs are combined with weighted **Reciprocal Rank Fusion**:

```text
score(doc) = Σ over legs that found it:  weight_leg / (60 + rank_in_leg)
```

That is where the numbers above come from: a document ranked 1 by both legs scores
`1/61 + 1/61 = 0.032786…`; one found only by the lexical leg at rank 1 scores
`1/61 = 0.016393…`.

**Only ranks are fused. Distances never are, and there is nowhere in the fusion code
to put one.** Two vector partitions are two different embedding spaces: a cosine
distance of 0.31 from a text tower and 0.31 from an image tower are not the same
quantity, any more than 0.31 metres is 0.31 seconds. Every arithmetic combination of
them is finite, every resulting ranking is plausible, and nothing logs a fault. Ranks
are comparable across legs; measurements are not.

The distances still reach you — as `vector_distance`, beside `embedding_kind` so you
can tell which scale it is on. Reporting a measurement is fine; fusing two
incommensurable ones is not.

Fusion is over **N legs**, not two. `kind => 'all'` runs one leg per embedding
partition and every vector leg carries the caller's full `vector_weight`, so a
document found by two towers outranks one found by either alone — which is the point
of asking for both.

### Turning a leg off

`vector_weight => 0` skips the vector leg entirely, including embedding-provider
resolution — so a tenant with no embedder configured can still run it:

```sql
SELECT path, score, fulltext_rank, vector_rank
FROM HYBRID_SEARCH('tarpaulin', 5, workspaces => 'library', vector_weight => 0);
-- /winter-layup  0.016393…  1  NULL
```

`fulltext_weight => 0` skips the lexical leg. Both at zero is refused rather than
returning nothing:

```sql
SELECT path FROM HYBRID_SEARCH('tarpaulin', 5, workspaces => 'library',
                               vector_weight => 0, fulltext_weight => 0);
-- ERROR: HYBRID_SEARCH: fulltext_weight and vector_weight are both 0, so neither
--        half of the search would run. Set at least one above 0.
```

## Cross-lingual retrieval

With a multilingual embedding model, a query in one language finds documents in
another, with no shared tokens and no translation step. All four documents below are
English; all three queries are German:

```sql
SELECT path, vector_distance
FROM HYBRID_SEARCH('Wartung von Segelbooten im Winter', 5,
                   workspaces => 'library', fulltext_weight => 0);
-- /winter-layup  0.3936
```

```sql
SELECT path, vector_distance
FROM HYBRID_SEARCH('Wie mahle ich Kaffeebohnen?', 5,
                   workspaces => 'library', fulltext_weight => 0);
-- /espresso  0.4240
```

```sql
SELECT path, vector_distance
FROM HYBRID_SEARCH('Zinsen für einen Immobilienkredit', 5,
                   workspaces => 'library', fulltext_weight => 0);
-- /mortgage  0.3905
```

Three for three, each with a wide margin over the next candidate. Two caveats, both
measured:

**The model matters, and it is not a tuning detail.** Run against the same corpus,
`bge-m3` (1024d, multilingual) answered 3/3 with roughly 0.3 cosine of separation
between right and wrong. `nomic-embed-text` (768d, English-first) answered 2/3, and on
the query it got right its top three scores spanned 0.440 / 0.416 / 0.411 — a 0.03
spread. That is not ranking, it is a coin flip, and it fails silently: no error, no log
line, just recall that quietly feels bad. Pick a multilingual model for a multilingual
corpus.

**Turn the lexical leg off for a cross-lingual query.** The lexical leg is
typo-tolerant (edit distance 1, prefix-matched), which is what you want for
`Datenbnak → Datenbank` and emphatically not what you want when the query is in
another language. Short German function words fuzzy-match English stems:

```sql
SELECT path FROM FULLTEXT_SEARCH('im', 'en', workspaces => 'library');
-- /mortgage, /winter-layup, /brake-pads, /espresso        -- all four
SELECT path FROM FULLTEXT_SEARCH('Segelbooten', 'en', workspaces => 'library');
-- (no rows)
```

So `HYBRID_SEARCH('Wartung von Segelbooten im Winter', …)` with default weights
returns the right document first and then three passengers. `fulltext_weight => 0`
removes them.

## `EMBEDDING()`, `<=>`, `VECTOR_OF()` and bound vectors

### The query vector

`KNN`'s first argument accepts five forms:

```sql
KNN('some text')                        -- embedded with the tenant's provider
KNN(EMBEDDING('some text'))             -- identical; the wrapper is unwrapped
KNN(ARRAY[0.1, 0.2, …])                 -- a vector literal
KNN('[0.1, 0.2, …]')                    -- the pgvector text form
KNN(VECTOR_OF('library:/winter-layup')) -- a node's own stored vector
```

The last two are what make **binding a vector as a parameter** work. `$1` never
reaches the parser — parameters are substituted into the SQL text first — and the HTTP
and pgwire substituters render a bound JSON array as `'[0.1,0.2]'` while the functions
runtime renders it as `ARRAY[…]`. Both land on the same vector:

```json
POST /api/sql/{repo}/{branch}
{
  "sql": "SELECT path, vector_distance FROM KNN($1, 3, workspaces => 'library')",
  "params": [[0.0134, -0.0271, …]]          // 1024 floats
}
```

```text
/winter-layup  0.27467623353004456
```

which is the same distance the text form gives for the text those floats came from:

```sql
SELECT path, vector_distance FROM KNN('storing a yacht over winter', 3,
                                      workspaces => 'library');
-- /winter-layup  0.27467623353004456
```

A vector of the wrong width is refused rather than degraded:

```text
ERROR: vector leg over partition 7TKpxhrIUdAT failed: Query dimension mismatch:
       expected 1024, got 8. The result would have been a full-text search
       reported as a hybrid one, so the statement fails instead.
```

### `VECTOR_OF(node_ref [, chunk])` — find things like this one

Reads a node's **stored** vector instead of re-encoding anything, so there is no
embedding call and the comparison is exact:

```sql
SELECT path, vector_distance
FROM KNN(VECTOR_OF('library:/winter-layup'), 4, workspaces => 'ALL READABLE');
```

```text
/post-winter   0.3351
/brake-pads    0.5427
/onboarding    0.5595
/expenses      0.5648
```

**The source node is excluded from its own results.** It is by definition its own
nearest neighbour, and a `LIMIT 10` should not spend a slot restating the question.

The reference grammar mirrors `REFERENCES(...)`: `'ws:/path'`, `'ws:<node-id>'`, and
`'ws:/path#spec'` for a named embedding spec. A reference that names no node is an
error, not an empty result:

```sql
SELECT path FROM KNN(VECTOR_OF('library:/nope'), 3, workspaces => 'library');
-- ERROR: VECTOR_OF('library:/nope'): no such node in workspace 'library'.
```

The **workspace prefix is required** —
embeddings are keyed by the workspace the node lives in, which is not necessarily one
of the workspaces being searched:

```sql
SELECT path FROM KNN(VECTOR_OF('/winter-layup'), 4, workspaces => 'library');
-- ERROR: VECTOR_OF('/winter-layup'): the reference must name a workspace, as
--        'workspace:/path' or 'workspace:<node-id>'. Embeddings are keyed by the
--        workspace the node lives in, which is not necessarily one of the
--        workspaces being searched, so it cannot be inferred.
```

`VECTOR_OF` is `KNN`-only, for the same reason a raw vector is: it has no lexical
surface, so a `HYBRID_SEARCH` built on it would be a vector-only search reported as
hybrid.

```sql
SELECT * FROM HYBRID_SEARCH(VECTOR_OF('library:/winter-layup'), 4, workspaces => 'library');
-- ERROR: HYBRID_SEARCH: VECTOR_OF(...) is a vector, and HYBRID_SEARCH needs text
--        for its full-text half. Use KNN(VECTOR_OF(...)) for similar-to-this-node
--        search.
```

For a chunked document, pass the chunk — a document's chunks are different vectors and
chunk 0 is an arbitrary one to pick:

```sql
SELECT path, vector_distance
FROM KNN(VECTOR_OF('library:/handbook-long', 12), 3, workspaces => 'library');
-- /brake-pads    0.5150
-- /espresso      0.5499
-- /winter-layup  0.5898
```

### The `<=>` operator

`<=>` is cosine distance, usable in an ordinary `SELECT … ORDER BY … LIMIT`:

```sql
SELECT path, embedding <=> EMBEDDING('storing a yacht over winter') AS distance
FROM 'library'
WHERE embedding <=> EMBEDDING('storing a yacht over winter') < 0.705
ORDER BY distance LIMIT 5;
```

```text
/winter-layup  0.27467623353004456
/brake-pads    0.6138415932655334
/mortgage      0.7008225917816162
```

It composes with a `WHERE` on ordinary columns, and the predicate is pushed into the
index scan:

```sql
SELECT path, embedding <=> EMBEDDING('storing a yacht over winter') AS distance
FROM 'library' WHERE node_type = 'kb:Doc' ORDER BY distance LIMIT 3;
-- /winter-layup  kb:Doc  0.27467623353004456
```

:::caution The bare form is capped at 0.6
Without a distance predicate, `ORDER BY embedding <=> EMBEDDING(…)` applies the
engine's default cutoff of **0.6** and silently drops everything beyond it — the query
above returns only `/winter-layup` if you delete its `WHERE`. Raising the tenant's
`default_max_distance` does **not** change the bare form. Either write the threshold
into the query (`WHERE embedding <=> … < 0.9`) or use `KNN(…, max_distance => 0.9)`,
which takes it as an argument.
:::

## `kind =>` and multi-modal fusion

`cf::EMBEDDINGS` has always carried a *kind* — `T` for text, `I` for image — and a
branch holds one HNSW index per embedding space. `kind` selects which of them the
vector leg reads:

| `kind` | Legs |
|---|---|
| `'text'` | the text partition only — **the default** |
| `'image'` | the image partition only |
| `'all'` | every configured partition, one leg each, rank-fused |

The default is `'text'` rather than `'all'` deliberately. If it were `'all'`, then on
the day an image tower first writes a vector every existing query would silently start
fusing a second corpus in: new rows in every result set, no error, no change to any
query text, and a `LIMIT 10` now spending slots on pictures. Breadth is opt-in and says
so in the query, exactly as `workspaces => 'ALL READABLE'` does.

Asking for a partition that does not exist is an error, not an empty result:

```sql
SELECT path FROM KNN('storing a yacht over winter', 5,
                     workspaces => 'library', kind => 'image');
-- ERROR: kind => 'image' selects no vector index: this branch has no image
--        embedding partition. An empty leg reported as a search is
--        indistinguishable from a corpus that matched nothing, so the statement
--        fails instead.
```

:::note Image embeddings are not shipped yet
The query surface, the key layout, the partitioning and the N-leg fusion are all in
place and `kind => 'image'` / `'all'` are live. What does **not** exist yet is anything
that *writes* an image vector — there is no image embedding tower. Until one lands,
`kind => 'image'` always errors as above, and `kind => 'all'` resolves to the text
partition alone.
:::

## Chunking, and which part of a document answered

A long document is split into chunks and each chunk is embedded separately, so a
hundred-page handbook does not collapse into one averaged point. `chunk_index` reports
which chunk matched:

```sql
SELECT path, chunk_index, vector_distance
FROM KNN('how often should standing rigging be replaced', 5, workspaces => 'library');
-- /handbook-long  6   0.2612
-- /brake-pads     0   0.5357

SELECT path, chunk_index, vector_distance
FROM KNN('what happens when the water pump impeller sheds a vane', 5, workspaces => 'library');
-- /handbook-long  12  0.3536
-- /brake-pads     0   0.5990

SELECT path, chunk_index, vector_distance
FROM KNN('how deep can I discharge a lead acid battery', 5, workspaces => 'library');
-- /handbook-long  16  0.4538
```

One document, three questions, three different chunks. A RAG caller needs
`chunk_index` to know *where* in a long document the answer lives.

## Branch scoping

Vectors and lexical documents are both indexed **per branch**: the `cf::EMBEDDINGS` key
carries the branch as its third segment and the HNSW index key is
`{tenant}/{repo}/{branch}`. Working content on `main` and published content on
`publish` are genuinely separate corpora, and the branch you query is the branch in the
SQL endpoint's path.

Publishing a node with a `__branch` write carries it into the target branch's indexes,
both legs, with no extra step:

```sql
-- on main
INSERT INTO 'library' (__branch, path, name, node_type, properties)
VALUES ('publish', '/winter-layup', 'winter-layup', 'kb:Doc', '{…}'::jsonb);
```

```sql
-- POST /api/sql/kb/publish
SELECT path, vector_distance FROM KNN('storing a yacht over winter', 3,
                                      workspaces => 'library');
-- /winter-layup  0.27467623353004456

SELECT path FROM FULLTEXT_SEARCH('tarpaulin', 'en', workspaces => 'library');
-- /winter-layup

VERIFY VECTOR INDEX;
-- consistent  hnsw_count 1  storage_count 1
```

and content that was never published stays out of the published corpus:

```sql
-- POST /api/sql/kb/publish
SELECT path FROM KNN('grinding coffee beans', 3, workspaces => 'library');
-- (no rows)

-- POST /api/sql/kb/main
SELECT path FROM KNN('grinding coffee beans', 3, workspaces => 'library');
-- /espresso, /brake-pads
```

The node type must exist on the target branch, as for any write.

## Configuring embeddings

### Which fields become vectors

Nothing is embedded unless a NodeType field says so. In SQL DDL:

```sql
CREATE NODETYPE 'kb:Doc' PROPERTIES (
  title         String FULLTEXT VECTOR,
  body          String REQUIRED VECTOR FULLTEXT,
  internal_note String                      -- neither indexed nor embedded
);
```

`FULLTEXT` puts the field in the lexical index, `VECTOR` in the embedding. They are
independent, and a field marked neither is invisible to both.

:::caution Both legs must cover the same fields
If a field is `FULLTEXT` but not `VECTOR`, the lexical leg can return documents the
vector leg has never seen, and vice versa. RRF then systematically favours whichever
leg indexes more of the corpus — a ranking bias with no error attached to it. Mark the
searchable body text with both.
:::

### The embedder

Per tenant, over SQL:

```sql
ALTER EMBEDDING CONFIG
  SET PROVIDER    = 'ollama'
  SET MODEL       = 'bge-m3'
  SET DIMENSIONS  = 1024
  SET BASE_URL    = 'http://127.0.0.1:11434'
  SET API_KEY     = 'not-used-by-ollama'
  SET INCLUDE_NAME = 'false'
  SET INCLUDE_PATH = 'false'
  SET ENABLED     = 'true';
-- Embedding configuration updated
```

Every key `ALTER EMBEDDING CONFIG` accepts:

| Key | Notes |
|---|---|
| `PROVIDER` | `OpenAI` \| `Claude` (Voyage) \| `Ollama` \| `HuggingFace` |
| `MODEL` | the provider's model id |
| `DIMENSIONS` | must match the model — `bge-m3` is 1024, `nomic-embed-text` is 768 |
| `BASE_URL` | any OpenAI-compatible endpoint; empty string clears it |
| `API_KEY` | encrypted at rest; never returned |
| `ENABLED` | `true` / `false` |
| `INCLUDE_NAME`, `INCLUDE_PATH` | fold the node's name / path into the embedded text |
| `DEFAULT_MAX_DISTANCE` | the tenant default for the `KNN` / `HYBRID_SEARCH` cutoff; `'default'` restores 0.6 |
| `DISTANCE_METRIC` | changing it requires `REBUILD VECTOR INDEX` |
| `MAX_EMBEDDINGS_PER_REPO` | integer or `'unlimited'` |

Read it back, and test that the job will actually succeed:

```sql
SHOW EMBEDDING CONFIG;
```

```text
enabled                  true
provider                 Ollama
model                    bge-m3
dimensions               1024
has_api_key              true
base_url                 http://127.0.0.1:11434
include_name             false
include_path             false
default_max_distance     0.60 (default)
distance_metric          Cosine
max_embeddings_per_repo  unlimited
```

```sql
TEST EMBEDDING CONNECTION;
-- result "Connection successful"  dimensions 1024  model bge-m3  success true
```

`TEST EMBEDDING CONNECTION` resolves the provider through the same resolver the
embedding job uses and reports the **dimensions the endpoint actually returned**, so a
green result here means the job will work. Compare that number against your
`DIMENSIONS` setting — a mismatch is the one misconfiguration that produces plausible
rankings over nonsense.

### Bring your own inference server

`BASE_URL` is not Ollama-specific. Any endpoint that speaks the OpenAI embeddings API
works — a self-hosted vLLM or TEI, an internal gateway, a regional endpoint — so a
tenant can keep every vector inside its own network:

```sql
ALTER EMBEDDING CONFIG
  SET PROVIDER   = 'openai'
  SET BASE_URL   = 'https://inference.internal.example.com/v1'
  SET MODEL      = 'bge-m3'
  SET DIMENSIONS = 1024
  SET API_KEY    = '…';
```

### Chunking

Chunking is per tenant and is configured over the REST endpoint (there is no
`ALTER EMBEDDING CONFIG` key for it):

```json
POST /api/tenants/{tenant_id}/embeddings/config
{
  "enabled": true,
  "provider": "Ollama",
  "model": "bge-m3",
  "dimensions": 1024,
  "base_url": "http://127.0.0.1:11434",
  "api_key_plain": "not-used-by-ollama",
  "include_name": false,
  "include_path": false,
  "max_embeddings_per_repo": null,
  "chunking": {
    "chunk_size": 120,
    "overlap": { "type": "Tokens", "value": 24 },
    "splitter": "recursive"
  }
}
```

`GET` on the same path returns it (never the API key):

```json
{ "model": "bge-m3", "dimensions": 1024,
  "chunking": { "chunk_size": 120, "overlap": { "type": "Tokens", "value": 24 },
                "splitter": "recursive" },
  "quantization": "F32", "base_url": "http://127.0.0.1:11434" }
```

| Field | Values | Default |
|---|---|---|
| `chunk_size` | target chunk size in **tokens** | 256 |
| `overlap` | `{"type":"Tokens","value":n}` or `{"type":"Percentage","value":0.2}` (clamped to 0.5) | 64 tokens |
| `splitter` | `recursive` (paragraphs → sentences → words), `fixed_size`, `markdown`, `code` | `recursive` |
| `tokenizer_id` | optional; defaults from the embedding model | — |

Omitting `chunking` entirely disables it: one embedding per node, whatever its length.
`quantization` (`F32` \| `F16` \| `Int8`) is on the same payload and takes effect on
the next index build — the scalar kind is baked into the graph, so an existing index
keeps the precision it was written with until a rebuild.

## Operating the index

### `SHOW VECTOR INDEX HEALTH`

One row per **partition**, because a branch holds one index per embedding space:

```sql
SHOW VECTOR INDEX HEALTH;
```

```text
partition     queried  status     count  dimensions  memory_bytes  quantization  metric
7TKpxhrIUdAT  true     available  31     1024        20978352      F32           Cosine
```

The partition token is `{embedder_hash}{kind}` — the same bytes as segments 5 and 6 of
the storage key. Two rows here means two embedding spaces, which is normal during a
model change and is why fusion never mixes their distances.

### `VERIFY VECTOR INDEX`

Compares what the index holds against what storage holds, in the same unit — **one row
per vector**, i.e. per chunk, not per node:

```sql
VERIFY VECTOR INDEX;
-- status "consistent"  hnsw_count 31  storage_count 31
```

A mismatch names the repair:

```text
status "mismatch"  hnsw_count 9  storage_count 31  action "Run REBUILD VECTOR INDEX to fix"
```

### `REBUILD VECTOR INDEX`

Purges the partition's index and re-inserts every stored vector. It re-**indexes**; it
does not re-**embed**, so it costs no provider calls and needs no network:

```sql
REBUILD VECTOR INDEX;
-- Vector index rebuilt: 31 embeddings indexed
--   (workspaces: content-blog, content-news, handbook, library)
```

Run it after changing `DIMENSIONS`, `DISTANCE_METRIC` or `quantization`, and after a
`VERIFY` mismatch. It is idempotent and it covers every workspace on the branch. To
re-embed from source text — after a model change — use
`POST /api/admin/management/database/{tenant}/{repo}/vector/regenerate`, which requeues
the embedding jobs.

### The ~60 second snapshot lag

The HNSW index is served from memory and snapshotted to disk by a background task that
runs **every 60 seconds**. A write is searchable as soon as its embedding job drains —
the lag is not a query-visibility delay. What it means is that a process killed within
a minute of a write can lose that vector from the on-disk index while the RocksDB row
survives. The next embedding job for that node repairs it (the job asks both "is it
stored?" and "is it indexed?"), and `REBUILD VECTOR INDEX` repairs it wholesale.

If a freshly written document is not searchable, the embedding job has not drained yet.
Watch the job queue, not the snapshot.

:::caution A stale lexical reader looks exactly like row-level security
The Tantivy reader reloads on commit with a delay, so for a moment after a write the
lexical leg can be missing a document the vector leg already has. That is
indistinguishable from RLS filtering the row out. Rule out reader lag — re-run the
query a few seconds later — before investigating permissions.
:::

## Other surfaces

The HTTP hybrid-search endpoint and the MCP `search_nodes` tool build their calls
through the same `SearchArgs` constructor the SQL parser produces, so they share the
scope resolver, the leg dispatch, the fusion and the RLS pass — a scope string means
the same thing wherever it is written. They expose less of it: `search_nodes` takes one
workspace and a `mode` of `fulltext` or `vector`, with no hybrid mode and no weights.
For fused ranking or a multi-workspace scope, use SQL.

## See also

- [Full-text search](./fulltext.md) — the lexical leg in detail: languages, stemming, query syntax
- [Branches](./branches.md) — `__branch`, and how a branch's indexes are isolated
