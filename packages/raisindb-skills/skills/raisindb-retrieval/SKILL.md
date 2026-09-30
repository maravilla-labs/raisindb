---
name: raisindb-retrieval
description: "Search and retrieval-augmented answers on RaisinDB: hybrid full-text + vector search, how documents are chunked and embedded, the built-in ask / search-documents / graph-context / extract-entities functions, db.search() and db.ask() in the JS SDK, citations and grounding, and knowledge-graph retrieval over RELATE edges. Use this whenever the user wants search, semantic or 'smart' search, a search box, site search, a chatbot or assistant that answers from their own content, RAG, 'ask my documents', citations, embeddings, chunking, vector search, KNN, HYBRID_SEARCH, or anything about making uploaded PDFs and documents findable — even if they only say 'it should find the right page' or 'answer questions about our handbook'."
---

# Search & retrieval

Three retrieval legs, built from nodes you already author:

| | Finds | Built from |
|---|---|---|
| **Full-text** | The words that are there | Fields marked `Fulltext` |
| **Vector** | Text that *means* the same, in any wording | Fields marked `Vector` |
| **Graph** | How things relate | `RELATE` edges (see `references/graph-rag.md`) |

Full-text and vector are **one engine**. `HYBRID_SEARCH` runs both and fuses by
rank; `KNN` is that engine with the lexical leg off; `FULLTEXT_SEARCH` with the
vector leg off. Learn the arguments once and all three follow.

Versions, because these are recent and the answer changes:

| Feature | Needs |
|---|---|
| Automatic document chunking, `granularity => 'chunk'`, the built-in functions, `NEIGHBORS('ws:/path', …)` | server **v0.6.34+** |
| `ask` / `search-documents` in Rust: `paths`, `include_kinds`, `locale`, expansion, the claim check, lexical-only passages | server **after v0.6.47** (ai-tools 1.0.37) |
| `db.search()` / `db.ask()` on the WebSocket client | **@raisindb/client 0.5.4+** |
| `db.search()` / `db.ask()` on the Node-safe HTTP client | **@raisindb/client 0.5.5+** |

The last row matters more than it looks: a route handler — where retrieval
belongs, because the browser must not hold a repository credential — is exactly
where the HTTP client is used. On 0.5.4 those methods exist on the WebSocket
client only, so code written against the advice fails on the client it names.

## 1. What gets indexed

The NodeType decides, per field:

```yaml
properties:
  - name: title
    type: String
    index: [Fulltext, Vector]
  - name: body
    type: String
    index: [Fulltext, Vector]
```

**Mark both legs, or ranking skews silently.** Fusion assumes the legs see the
same corpus. A field that is `Fulltext` but not `Vector` puts documents in one
leg's reach and not the other's, and the fused order then favours the leg with
wider coverage — no error, no log line, just an order nobody chose.

Uploaded files carry a second body: **extracted text**, on the asset node as
`__extracted_text`, indexed like any other content.

**What open-source RaisinDB extracts natively is narrow, and this trips people
up:** `application/pdf`, images (via OCR) and `text/*`. That is deliberate —
LibreOffice must not live in the server process.

A `.docx`, `.pptx` or `.xlsx` is therefore NOT searchable on a stock server. It
is recorded as `__extract_status = 'unsupported'` or `'delegated'` and waits for
a converter that open-source does not ship. The converter is the Maravilla
**media plugin**, part of the commercial Studio product; it hands markdown back
through `raisin.assets.setExtractedText`, after which everything here applies
identically — core's own PDF path also produces markdown, so both sides of that
seam are the same. If the user has Studio, point them at its `studio-asset-pipeline`
and `studio-search` skills. If they do not, say so plainly rather than writing a
chunker or a converter in a function: an asset stuck on `delegated` is waiting
for a product, not for code.

Audit what you actually have before promising search over a mixed pile:

```sql
SELECT properties->>'__extract_status'::String AS status, COUNT(*)
FROM 'library' GROUP BY 1;
```

## 2. Chunking

Long text is split into chunks, each embedded separately, so a question about
page 30 is not answered with page 2.

- **Document bodies are chunked automatically** — 512 tokens, 64 overlap — with
  nothing to switch on. The old default was no chunking, which made a 40-page
  contract ONE vector: close to every query, specific to none, with a healthy
  index and no error anywhere.
- **A node's own fields are not chunked** unless configured. Titles and captions
  are short, and splitting them makes every fragment match everything.

Override per kind of content with a **processing rule** (admin console →
Processing Rules, or the processing-rules API), matched on node type, path,
workspace or mimetype:

```json
{ "chunk_size": 512,
  "overlap": { "type": "Tokens", "value": 64 },
  "splitter": "recursive",
  "tokenizer_id": "text-embedding-3-small" }
```

**Set `tokenizer_id` when writing rules through the API.** Sizes count tokens
only when a tokenizer is named, and characters when it is not — so
`chunk_size: 512` without one quietly means 512 characters, about a paragraph.
The console fills it in; raw API calls do not.

Vector search needs an embedder configured once per tenant (provider, model,
key). Without one, full-text still works and the vector leg is simply off.

## 3. Querying

```sql
SELECT path, chunk_index, chunk_text, score
FROM HYBRID_SEARCH('how much notice to terminate', 5,
                   workspaces => 'handbook',
                   granularity => 'chunk');
```

Arguments: `(query, limit)` positionally, then named — `workspaces`,
`granularity`, `language`, `vector_weight`, `fulltext_weight`, `max_distance`,
`kind`.

**`granularity => 'chunk'` is what a RAG prompt wants.** It returns one row per
PASSAGE, so `LIMIT 5` means five passages and several may share a document. The
default (`'node'`) returns one row per document, so `LIMIT 5` means five
documents — right for a results page, wrong for a context window.

Columns worth knowing:

| Column | Meaning |
|---|---|
| `chunk_text` | The passage that matched — render this as the snippet |
| `chunk_text_source` | `exact` (the real passage), `excerpt` (a 200-char preview), `unavailable` |
| `chunk_index` | Where in the document. **`0` is a real chunk**, not "missing" |
| `score` | Fused rank score. Comparable within one result set only |
| `vector_rank`, `fulltext_rank` | NULL when that leg did not match |

Fusion is **rank-based**, never score-based: two vector partitions are two
embedding spaces, and combining their distances produces plausible nonsense.

### The workspace scope is required

One string, four forms:

| Form | Example |
|---|---|
| One name | `'handbook'` |
| A list | `'handbook, policies, stories'` |
| A glob | `'content-*'` (one token, never mixed into a list) |
| Everything readable | `'ALL READABLE'` |

**A name is an assertion; a glob is a question.** A listed name that does not
resolve is an ERROR — you meant it, so a typo is reported rather than quietly
searching the rest. A glob matching nothing is fine and returns no rows.

`'*'` and `'ALL'` are rejected on purpose: `'*'` reads as "unscoped" and the eye
slides past it, while two uppercase words appear in no other context.

Row-level security applies to results either way. **The scope is intent, not
permission** — it decides which corpus an answer is drawn from. That matters
most on a public site, where the identity your route authenticates as can
usually read more than the published pages.

## 4. The built-in functions

Four ship in the `ai-tools` package as ordinary function nodes — read them, copy
them, replace them:

| Path | Does |
|---|---|
| `/lib/raisin/ai/search-documents` | Passages with `path`, `node_id`, `chunk_index`, `workspace`, `node_type`, `kind`, `title`, `snippet` |
| `/lib/raisin/ai/ask` | Expand → retrieve → grade → rewrite once → answer → check each claim, with citations |
| `/lib/raisin/ai/graph-context` | Seeds by meaning, walks the graph outward |
| `/lib/raisin/ai/extract-entities` | Builds the graph from a document (see `references/graph-rag.md`) |

`search-documents` and `ask` are one Rust WebAssembly component (source in
`tooling/ai-tools-rag`, rebuilt with `make ai-tools-rag`), shipped inside the
server; `ask` runs retrieval in-process. Their paths and output shapes are the
ones the JavaScript versions had, plus fields.

`ask` returns:

```json
{ "answer": "Either party may terminate on thirty days written notice [1].",
  "grounded": true,
  "citations": [{ "marker": 1, "path": "/contracts/msa", "workspace": "docs",
                  "node_type": "raisin:Document", "kind": "page", "title": "MSA",
                  "snippet": "Either party may terminate…", "chunk_index": 0,
                  "text_is_exact": true, "matched": "both", "source": "chunk",
                  "locale": null, "url_hint": null }],
  "attempts": [{ "query": "…", "passages": 8, "expansions": ["…"] }],
  "verification": "passed" }
```

**`grounded: false` means no answer.** When retrieval finds nothing, the model
is never called — it would answer from its own training, fluently, with nothing
marking it invented. When the claim check drops every sentence that cites a
passage (`verification: "rejected"`), the result is `grounded: false` too, with
the dropped sentences in `dropped_claims`. Branch on this flag before showing an
answer; `verification: "unavailable"` means the check itself failed and the
draft is unchecked.

### Scoping a site's chatbot

A Studio `stories` workspace holds several sites, `/demo` and blueprints; the
library holds images whose captions embed closer to a short question than any
page. The call a site makes:

```js
ask({ question,
      workspaces: ['stories', 'assets'],   // a list works (it used to mean ALL READABLE)
      paths: ['/bap'],                     // this site only, in every workspace
      locale: 'fr',                        // passages/titles/url_hint from the FR overlay
      base_language: 'de',                 // content language: lexical leg + expansion target
      include_kinds: ['page', 'document'] })
```

- `paths` — prefixes applied per workspace (`'assets:/bap'` binds one to one
  workspace). They reach the engine as a `WHERE` over `HYBRID_SEARCH`, which is
  evaluated inside its fetch loop, so the limit counts in-scope rows.
- `include_kinds` — `page`, `document`, `image`, `media`, `all`. **Default:
  everything but `image`** — the one change for callers that pass nothing new.
  `node_types` / `exclude_node_types` narrow by type.
- `locale` — retrieval stays locale-blind (overlays are in neither index); the
  multilingual vector leg finds the base page, and its text comes back from the
  overlay where one exists.
- `expand` (ask, default on up to eight words) — one model call widens the
  question with the words documents use to STATE the answer: "CEO" →
  "Geschäftsführer", "who owns it" → shareholders, shares. Searched on the
  lexical leg only; never shown to the model as facts.
- `verify` (ask, default on) — one model call judges each sentence against the
  passages; unsupported ones are dropped. It exists because a model asked "Wem
  gehört der Flughafen?" without the shareholder passage answered by making the
  towns on a directions page the owners.
- `candidates` (default 24) — each leg's draw before filters, fusion and the
  per-document cap (`max_per_document`, 2 in `ask`). It is the retrieval's
  cost: the engine draws 20x it per leg and redraws wider when a scope leaves
  it short.
- **Confidence gating.** When the question's own words are in the top three
  passages and the best one matched on words, `ask` skips expansion and the
  grader (`attempts[0].confident`, `timings.grade_skipped`). Otherwise
  expansion (short questions) adds ONE full-text query to the retrieval it
  already has — no second hybrid search, no second query embedding.
- **Term-coverage rerank.** After passage text (tables included) is read,
  candidates are re-ordered by how many of the question's distinctive words
  they contain (weighted by rarity among the candidates), so the Parken page
  whose table has "Wochentarif" beats AGB PDFs that only share "Parken".
- `ask` and `search-documents` run synchronously too (`execution_mode:
  both`): a request/response caller need not poll.
- The claim check also drops sentences that lean on a dropped one ("Diese
  wird … genannt" after the sentence naming "diese" went).
- `rewrite` (ask, default on) — the grader call and its one retry;
  `rewrite: false`, `expand: false` and `verify: false` leave `ask` with the
  answer call alone. The host gateway is synchronous, so these calls run in
  sequence; every result carries `timings` (ms per SQL leg, passage reads,
  and each model call) to show which one to switch off.
- **Tables are passage text.** A page passage carries the page's tables
  (`{rows: [[…]]}` grids anywhere in its properties, e.g. a tariff block),
  rendered row by row with their column headers, in the reader's locale —
  the chunk the vector leg matched never contained them. Snippets and
  passages are plain text (HTML stripped); citation markers come back as
  `[n]` whatever style the model wrote (`【1】`, `【1†L3】`).

A lexical-only hit (the vector leg missed it) has NULL `chunk_text`; it now
gets a passage cut around the matched words (`source: "excerpt"`) instead of
being dropped. Without an embedder, retrieval runs full-text only and says
`mode: "fulltext"`.

**`attempts` shows the retry.** When the graded passages do not answer the
question, the query is rewritten in the documents' own vocabulary and tried once
more. "How much warning before we get kicked out?" finds nothing; "termination
written notice period" finds the clause.

**`text_is_exact: false`** means only a preview was available. Point a reader at
the document; do not quote it as a whole passage.

## 5. From an app (JS SDK)

```ts
const passages = await db.search('how much notice to terminate', {
  workspaces: 'handbook',
});
passages[0].text;        // the passage
passages[0].chunkIndex;  // where in the document

const { answer, citations, grounded } = await db.ask(
  'How much notice do we have to give?',
  { workspaces: 'handbook, policies' },
);
```

`workspaces` is **required** on both — on a public page it is the argument to
think hardest about.

**Use these, not `db.functions().invoke('ask', …)`.** `invoke` is the escape
hatch for functions *you* wrote: it names the callee in a string resolved at run
time, makes every call site rebuild the arguments by hand, and freezes the
transport. These methods are typed and own their shapes.

Both clients have them from **0.5.5** — the WebSocket `Database` and the
Node-safe `HttpDatabase` (0.5.4 has the WebSocket one only). The credential
stays server-side either way: a browser cannot hold a key that reads the
repository, so retrieval runs in a route handler and the browser calls that
route.

## 6. A chatbot is an agent with these tools

```yaml
node_type: raisin:AIAgent
properties:
  provider: openai     # REQUIRED — a provider configured on the tenant
  model: gpt-4o        # REQUIRED
  tools:
    - /lib/raisin/ai/search-documents
    - /lib/raisin/ai/ask
    - /lib/raisin/ai/graph-context
```

`/agents/research-assistant` ships wired this way as a worked example. For the
chat pipeline, conversations and streaming, see the `raisindb-messaging-agents`
skill.

The prompt decides whether it uses the tools. State the refusal as the *correct*
outcome — "when the documents do not cover something, say so plainly; that is a
correct answer" — because a model treats a grudging fallback as something to
avoid, and will fill the gap fluently instead.

## 7. Traps

These all fail silently. They cost real debugging time.

- **Inside a function, call another function with `raisin.functions.call(path, args)`.**
  `functions.execute` is the AI-tool-call form: it stamps a `raisin:AIToolCall`
  node from its third argument, so outside that context it fails "Node not
  found" before the callee runs.
- **Multi-hop graph walks address nodes by `id`, not `path`.** A bare path
  resolves in the default workspace only, and a neighbour row does not say which
  workspace it came from. `NEIGHBORS('ws:/path', …)` works from **v0.6.34** —
  before that the workspace-prefixed form silently matched nothing.
- **An agent missing `provider` or `model` is rejected at install time,
  quietly.** If your agent is not there, check those two first.
- **`chunk_index: 0`** is a real chunk. A falsy check drops it.
- **After upgrading to 0.6.34**, the first run re-embeds every extracted
  document once, because the chunking default changed the spec hash. One
  embedder call per chunk, then it is stable again.
- **Nothing found?** In order: is the field marked for the leg you are
  searching, is an embedder configured, and has the node been written since
  either changed? Writes are indexed on save; nothing re-indexes retroactively
  except a rebuild.
- **Full-text language is the repository's `default_language`.** Base content
  is indexed under it, and locale overlay text is not in the lexical index at
  all. On a `de` repository `FULLTEXT_SEARCH('flugplan', 'de')` finds the base
  pages; on a repository created with the default `en`, the same German pages
  sit under `'en'` until the default is changed
  (`raisindb repo languages <repo> --default de`, which queues the rebuild
  itself). For a visitor in another locale, search the base content
  semantically (`KNN` with a multilingual embedder) instead of lexically. Up to
  v0.6.45 a full-text `rebuild` / `reconcile` filed everything under `en`
  whatever the repository said; rebuild once on v0.6.46 or later.
- **Which repair?** `POST /api/admin/management/database/{tenant}/{repo}/…`:
  `fulltext/rebuild` after recreating a repository or changing its languages;
  `vector/rebuild` (or `REBUILD VECTOR INDEX`) re-adds the stored embeddings to
  the index, no embedder calls; `vector/regenerate` queues re-embedding for
  nodes whose stored vector has the wrong dimensions and, from v0.6.46, for
  every embedding-eligible node that has no stored embedding (e.g. its job
  died while the embedder was down), skipping nodes with a pending job
  (`?force=true`: every stored embedding too).
  An unreadable vector index file is set aside as `*.unreadable-<ts>` and
  rebuilt from the stored embeddings automatically.

## 8. Cost

- `ask` is up to four model calls: expansion (short questions), grader, answer,
  claim check. `expand: false` and `verify: false` take it back to two. `search-documents`
  costs no model call — reach for it when you want passages.
- Embedding happens on write, once per chunk. A bulk import is the moment to
  check provider rate limits.
- `extract-entities` is one call per *changed* document; unchanged documents
  cost nothing.

## Going further

- `references/graph-rag.md` — building a knowledge graph from documents,
  `RELATE` vs references, and walking it. Read it when the question is about how
  things relate rather than what a document says.
