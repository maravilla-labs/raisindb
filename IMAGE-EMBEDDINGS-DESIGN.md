# Image search in RaisinDB — design (revision 2)

Status: decision aid. Revised after two adversarial reviews, both of which
landed hits. **The recommendation changed.** Every code claim carries a
confidence tag: *(verified in code)* = I read the cited lines in this worktree
or in the vendored crate source; *(verified external)* = fetched from the HF /
crates.io API during this revision; *(inferred)* = reasoned from verified
facts; *(unverified)* = neither, and named as such.

## The answer, so nobody reads to the end for it

> **Ship OCR first, captioning second, and do not buy the HNSW partitioning
> work yet.**
>
> 1. **Image OCR into `extracted_text`** is the cheapest and, on half a CMS
>    corpus, the *best* image-search instrument. Zero new dependencies, zero
>    YAML change, zero weights, zero LLM bill, works air-gapped, and German is
>    a downloadable Tesseract language pack. It is one mime branch away.
> 2. **Captioning** rides bge-m3's proven cross-lingual margin — and the local
>    captioner is **already written and wired**, missing only its call site.
> 3. **A dedicated image tower** (UForm-ONNX or SigLIP-in-candle) buys visual
>    nuance and query-by-example. It is real, but it is third, and it is gated
>    on a three-arm measurement that does not exist yet.
> 4. **Phase 0 splits.** Three of its four items (`hnsw_transfer`, the moka
>    weigher, i8 quantization) are independently justified today and are about
>    a week. The fourth — index partitioning, two-thirds of the bill — buys
>    nothing until a second vector space actually exists. Buy it when the image
>    tower is commissioned, not before.

**What changed from revision 1, and why.** Revision 1 recommended
captioning-then-UForm and declared the entire candle branch dead on
multilingual grounds. Three corrections:

| revision 1 said | correction | evidence |
|---|---|---|
| "There is no multilingual CLIP in candle-transformers 0.9.2" — **bold, disqualifying** | **Withdrawn.** `models/siglip.rs` is in the pinned crate; its `Config`/`TextConfig`/`VisionConfig` all `#[derive(serde::Deserialize)]` with `#[serde(default)]` on every field, and `google/siglip-base-patch16-256-multilingual` deserializes into exactly that shape | *(verified in code + external)*, §3.3 |
| Local captioning is impossible; the caption must be a hosted VLM | **Withdrawn.** `raisin-ai/src/candle/moondream/` is a complete promptable captioner and `AssetProcessingHandler::generate_image_caption` already calls it. Only the call site is missing | *(verified in code)*, §1.3 |
| Image OCR listed in an inventory table and never analysed | **Promoted to item 1a.** It is the strongest instrument on the text-dominant half of a brand-asset library | *(verified in code)*, §2 |

Two smaller corrections, both of which make the document's own case
*stronger*: the quantization plumbing is not merely unused, it is
**configurable-but-inert in a shipped UI** (§5.5); and the moka weigher defect
is not the one revision 1 described (§5.5).

---

## 1. What exists today, honestly

### 1.1 The caption fields carry no `Vector` flag *(verified in code)*

`crates/raisin-core/global_nodetypes/raisin_asset.yaml` (`version: 4`):

```yaml
  - name: title           index: [Fulltext]          # :25-28
  - name: description     index: [Fulltext]          # :60-63
  - name: alt_text        (no index key at all)      # :64-68
  - name: keywords        index: [Fulltext]          # :69-72
  - name: extracted_text  index: [Fulltext, Vector]  # :101-104  <-- the only one
```

The embedding job collects a top-level property **iff** it is in the resolved
`Vector` plan (`jobs/handlers/embedding/content_extraction.rs:31-105`;
`collect_plan_values` gates on `plan.top_level_props`). So a generated caption
is fulltext-searchable and **invisible to every vector query**.

`extracted_text` being the one `Vector`-flagged property is the single most
consequential fact in this document, and it is what makes §2's conclusion fall
out: **anything that can put text into `extracted_text` inherits the whole
retrieval stack for free.**

### 1.2 An image embedding IS computed on every upload, and thrown away *(verified in code)*

`AssetProcessingHandler::process_image_embedding` (`asset_processing/handler.rs:534-565`)
loads CLIP, produces a 512-dim vector, and parks it in
`AssetProcessingResult.image_embedding` (`types.rs:41-46`). Nothing reads that
field. No `EmbeddingData` is constructed; nothing reaches `cf::EMBEDDINGS` or
HNSW. `EmbeddingKind::Image` (`raisin-ai/src/config/embedder.rs:67-95`) is never
constructed anywhere — its only appearances are its own `to_key_char` /
`from_key_char`.

Worse, it is on by default. `ProcessingSettings::effective_tasks`
(`raisin-ai/src/rules/settings.rs:178-186`) is
`self.generate_image_embedding.unwrap_or(is_image)`, and the handler is
registered unconditionally (`storage/jobs/init_system/ai_handlers.rs`,
`create_asset_processing_handler` always returns `Some`). **With no processing
rules configured at all, every installation downloads ~600 MB of CLIP on the
first image upload and runs an inference per image for a discarded vector.**
See §4 for what that costs.

### 1.3 The local captioner exists and is unreachable *(verified in code)*

This is the finding revision 1 missed, and it changes the plan's shape.

- `crates/raisin-ai/src/candle/moondream/` — 953 lines, complete. Quantized
  `santiagomed/candle-moondream` (~1.8 GB), with `generate_alt_text`,
  `generate_description`, `generate_keywords`, `caption_with_options`
  (`moondream/mod.rs:49,146,271-436`). `candle/blip.rs` sits beside it.
- `AssetProcessingHandler::generate_image_caption` (`handler.rs:219-239`) and
  `generate_image_keywords` (`:241-259`) already load it via
  `get_or_load_captioner(model_id)` and already thread the console's
  `caption_model` and three prompts through.
- **Neither has a call site.** The handler's entire response to the six console
  settings is one `tracing::warn!` at `:567-585` saying captioning is disabled
  and to use trigger functions instead.

So "wire or delete the orphaned console controls" is not 2-3 days of deciding —
the implementation behind those controls is finished. Calling it is the cheapest
possible caption path, and it removes the air-gap objection revision 1 named as
captioning's one real weakness.

### 1.4 The CLIP text tower is fake, and the two CLIP call sites disagree *(verified in code)*

```rust
// crates/raisin-ai/src/candle/clip.rs:219-233
/// Note: This is a placeholder. In production, use the proper CLIP tokenizer.
fn tokenize_simple(&self, text: &str) -> CandleResult<Vec<u32>> {
    ... tokens.push((c as u32) % 49000 + 1000);
```

A text query through that is noise. The existing CLIP path cannot answer a text
query in **any** language, English included.

Revision 1 said fixing this means "someone has to write a real BPE tokenizer."
**That was overstated.** `tokenizers = "0.21"` is already a dependency of the
same `candle` feature (`raisin-ai/Cargo.toml:21-25,67`),
`tokenizers::Tokenizer::from_file` is already used in `candle/blip.rs:116` and
`candle/moondream/mod.rs:171`, and the downloader already fetches
`tokenizer.json` by name. Replacing `tokenize_simple` is wiring an existing
in-tree pattern. The substantive claim — today's CLIP path cannot serve a text
query — stands.

Two call sites already drifted: `handler.rs:84` hardcodes
`openai/clip-vit-base-patch32`; `candle/clip.rs:22` sets `DEFAULT_CLIP_MODEL =
laion/CLIP-ViT-B-32-laion2B-s34B-b79K`. This design adds no third site.

Also stale: `AIEmbedCallback`'s doc comment
(`raisin-functions/src/api/callbacks/service_ops.rs:94`) advertises an
`input_type: "image"` field that nothing parses; image-vs-text is decided by a
base64 heuristic. The shipped example function passes it and is ignored.

### 1.5 OCR is compiled in, proven reachable, and blocked by one classification *(verified in code)*

The finding that reorders the whole plan:

- `OcrProvider::ocr_image(&[u8], &OcrOptions)` takes **raw image bytes**
  (`raisin-ai/src/pdf/ocr.rs:76`), and `TesseractOcrProvider` implements it
  (`:229-284`).
- `raisin-server`'s default `ai` feature enables `raisin-ai/ocr`
  (`raisin-server/Cargo.toml:131,134`).
- `raisin-functions/src/runtime/bindings/methods/pdf.rs:211,229` already calls
  `get_default_ocr_provider().ocr_image(...)` in-process today. So the
  capability is compiled in and demonstrably reachable.
- `extracted_text` is **already** `[Fulltext, Vector]`, so text landing there
  needs no schema change at all.

The only obstacle is a classification: `rules/tasks.rs:133-137` declares
`image_ocr` as `TaskProvider::Plugin { method: "media.image.ocr" }`, and the
asset job passes `|_| false` to `plan_tasks`, so it is blocked as
`PluginMissing` — **even though `extract_text`, whose PDF path uses that same
provider, is `TaskProvider::Native` (`:78-80`).** That is an inconsistency, not
a constraint.

And the insertion point is already documented in-tree. `helpers.rs:205-241`:

```rust
/// # This is the whole vocabulary, in one place
/// ... Adding a mime type here is only half the change: [`process_extractable`]
/// must gain a branch that can actually read it ...
pub(crate) fn is_extractable_mime(mime_type: &Option<String>) -> bool {
    matches!(mime_type.as_deref(), Some("application/pdf"))
}
```

One vocabulary, one dispatch point, both already written down as the place to
widen. The result flows through the existing `persist_extracted_text`
(`handler.rs:407-478`) into the existing funnel. This is the opposite of the
mirrored-path bug class CLAUDE.md warns about.

**Two caveats that must ship with it, or it is a trap:**

- **`de` vs `deu` becomes load-bearing.** `get_default_ocr_provider()` returns
  `TesseractOcrProvider::english()` (`ocr.rs:291-296`), and
  `OcrOptions.languages` is `.join("+")`-ed and passed **verbatim** to Tesseract
  (`ocr.rs:242-246`) while the doc comment advertises ISO 639-1 (`'de'`).
  Tesseract wants `deu`. Revision 1 filed this as "adjacent"; putting OCR on the
  image path makes it the multilingual gate. *(verified in code)*
- **The `ocr` feature reaches the asset job only by workspace unification.**
  `crates/raisin-rocksdb/Cargo.toml:98` declares
  `raisin-ai = { features = ["huggingface", "candle"] }` — **not `ocr`**. It
  works today only because `raisin-server` builds with `ai` (which includes
  `ocr`) in the same workspace. `cargo test -p raisin-rocksdb` gets
  `NoOpOcrProvider`, and any build that drops the server's `ocr` feature loses
  image OCR **with no compile error**. Add an explicit `ocr` feature to
  `raisin-rocksdb` rather than inheriting one. *(verified in code)*

### 1.6 What raisindb does and does not do to an image *(verified in code)*

| operation | status |
|---|---|
| CLIP image embedding | computed, **discarded** (§1.2) |
| PDF text + OCR + first-page thumbnail | native, works |
| **image OCR** | provider compiled in and callable; **blocked by a `Plugin` classification** (§1.5) |
| **image captioning** | **fully implemented locally**; **no call site** (§1.3) |
| image resize / thumbnail | function temp-file API, or `media.image.resize` plugin. No typed `thumbnail` property exists |
| face detection | `media.image.detectFaces` plugin only |
| EXIF | nowhere (zero hits repo-wide) |
| content-hash dedup on upload | **not wired** — a re-upload re-extracts, re-captions and re-embeds (§7.3) |

Studio ships no asset trigger at all; `alt_text` / `description` / `tags` are
typed by hand in `AssetDetailsTab.svelte`.

---

## 2. The corpus question, answered — and it decides the order

The brief asked, seriously, whether image embedding is the right instrument for
a CMS corpus. It is not the first one.

**A Studio asset library splits into two halves.** *(inferred, from the shape of
brand-asset libraries; the split ratio is exactly what §9's measurement should
establish for a real tenant.)*

- **Text-dominant:** posters, slides, screenshots, ads, packaging, product
  shots with visible labels, certificates, scanned documents.
- **Photographic:** event photos, lifestyle, portraits, landscapes, textures.

The three instruments do not compete evenly across that split:

| | OCR → `extracted_text` | caption → `description`/`alt_text` | image vector |
|---|---|---|---|
| answers | "the poster with the headline *Sommerfest 2024*", a SKU, a campaign slug, a date | "the photo of the harbour at sunset" | "the image that looks like *this*", near-duplicates, style |
| text-dominant half | **wins outright** | paraphrases, loses the exact string | **structurally cannot read it** |
| photographic half | returns nothing | **wins** | wins on nuance, ties on subject |
| multilingual | Tesseract `deu` pack, then the string enters bge-m3's space | an English caption still matches German queries at bge-m3's 0.72-vs-0.33 margin, or caption in German natively | depends entirely on the tower's text side |
| other value | none | **accessibility (a real `alt`), SEO, an editor can read and fix it** | none outside search |
| air-gapped | yes | yes (moondream) / no (hosted VLM) | yes |
| marginal cost | ~0.2-1 s CPU | 10-90 s CPU local, or ~$0.002 hosted | ~50 ms-2 s CPU |

**Why a contrastive tower structurally loses the text-dominant half.** A
224x224 (UForm) or 256x256 (SigLIP) input with 16x16 patches gives roughly
14x14 to 16x16 tokens over the *whole* image. A poster's headline occupies a few
of them; its body copy occupies fractions of one. CLIP-family models are
documented to be weak at rendered text and are famously fooled by typographic
content. *(inferred from architecture plus published CLIP typographic-attack
results; not measured here.)* Meanwhile OCR returns the literal string a
marketer types into the search box. **On that half, the cheap instrument is also
the better one.**

**They are complements, not alternatives, and they compose for free**, because
all three land on the *same node*: OCR and captions become `extracted_text` /
`description` rows in the text partition, an image vector becomes an `I`-kind
row (§6.1), and RRF fuses ranks on `(workspace_id, node_id)`. A poster whose OCR
matched *and* whose caption matched ranks above either alone.

**Order follows directly:** OCR (cheapest, best on the common half, most
multilingual, air-gapped) → captioning (covers the photographic half, and pays
for itself in accessibility alone) → image tower (buys the residual: visual
nuance, query-by-example, near-duplicates).

---

## 3. If and when an image tower is bought: the option table

Revision 1's table had four rows and a bold disqualification. It now has six,
and the disqualification is narrower.

### 3.1 The multilingual requirement, restated

The owner's requirement is multilingual, and the project has *measured* evidence
that it matters: bge-m3 separates correct from incorrect German→English
retrieval by ~0.3 cosine (3/3), while nomic-embed-text coin-flips at a 0.03
spread (2/3) — `MEASURED-cross-lingual.md`. *(verified — measurement artifact
read during this revision.)* An English-only image tower would make image search
work in English and silently degrade in German. That remains disqualifying.

### 3.2 The table

| option | multilingual | new deps / binary | weights | dim | verdict |
|---|---|---|---|---|---|
| **A. UForm-multilingual via ONNX (`ort`)** | **PASS** — 21 languages, 250,037-token unigram SP vocab *(verified external)* | `ort` **has no stable release** (max `2.0.0-rc.13`, `max_stable_version = null`) *(verified external)*; ONNX Runtime static growth **unmeasured** | 222 MB: 87 MB image + 111 MB text ONNX + 24 MB tokenizer *(verified external, byte-exact)* | 256, Matryoshka-sliceable | viable, but pays a release-candidate C++ runtime and a downloader change |
| **B. candle CLIP / openclip** | **FAIL** — both default `vocab_size: 49408`, the OpenAI English BPE *(verified in vendored source)* | none | ~600 MB | 512 | **dead.** Also `ClipConfig` derives only `(Clone, Debug)` with one constructor `vit_base_patch32()`, and `ClipEmbedder::load_from_path`'s else-branch returns it for *any* model id — ViT-L weights would load into a ViT-B config *(verified in code)* |
| **B'. candle `chinese_clip`** | FAIL — ZH+EN only (BERT 21128 / 30522 vocabs) *(verified in vendored source)* | none | — | — | dead for European languages |
| **E. candle SigLIP multilingual** *(new)* | **PASS** — `google/siglip-base-patch16-256-multilingual`, WebLI with no language filter, 250,000-token mT5 SP vocab, Apache-2.0, not gated *(verified external)* | **ZERO.** candle-transformers is already in the default build; `tokenizers` 0.21 already a dep; **no downloader change** *(verified in code)* | **1.48 GB f32** in a single `model.safetensors` *(verified external)* | 768 | **the zero-dependency candidate. Must be in the spike.** |
| **C. VLM captioning → bge-m3** | **PASS, inherited and measured** | none (moondream already in tree) | 1.8 GB local, or 0 hosted | 1024 (existing) | **Stage 1.** Not an image tower at all — that is the point |
| **D. Hosted multimodal (Cohere Embed v4)** | PASS — 100+ languages, native Matryoshka *(vendor doc, unverified)* | none | none | 256-1536 | strongest on merit, weakest on egress / air-gap |

### 3.3 What I verified about SigLIP, since it overturns a bold claim

Read directly in
`~/.cargo/registry/src/index.crates.io-*/candle-transformers-0.9.2/src/models/siglip.rs`:

- `#[derive(serde::Deserialize, Clone, Debug)] pub struct TextConfig` (`:58-79`),
  `VisionConfig` (`:121`), `Config { text_config, vision_config }` (`:249-252`)
  — **every field carries `#[serde(default = ...)]`**, so an arbitrary HF
  `config.json` drives it. This is precisely the property `ClipConfig` lacks.
- `Model::get_text_features(&Tensor)` (`:777`) and `get_image_features(&Tensor)`
  (`:781`) — the whole API needed. `Model::new` also requires top-level
  `logit_scale` / `logit_bias` tensors (`:768-769`), which the HF `SiglipModel`
  checkpoint carries.

And the checkpoint, fetched from the HF API during this revision:

```
google/siglip-base-patch16-256-multilingual    apache-2.0, not gated
  config.json          348 B   {model_type: siglip, text_config.vocab_size: 250000,
                                vision_config.image_size: 256}
  model.safetensors    1,482,553,200 B
  tokenizer.json       16,350,281 B
```

Those three filenames are **exactly** what `download_standard_model`
(`raisin-ai/src/huggingface/registry/download.rs:146-210`) already fetches by
hardcoded name. *(verified in code.)* So option E needs no downloader change, no
new crate, and no binary growth — the four axes on which revision 1 costed
UForm at 3-4 weeks.

**But it is a trade, not a strict win.** The honest costs, all verified:

- **1.48 GB of f32 weights in one file**, dominated by the 250,000 x 768 text
  embedding table (~768 MB alone). Unlike UForm, the towers are **not separate
  files**, so an indexing-only node still downloads the whole thing; only
  resident memory can be trimmed (candle can mmap as F16, ~740 MB). *(verified
  external + inferred.)*
- **768-dim output vs UForm's 256** — 3x the index payload (§5.5's table).
- **candle CPU inference is materially slower than ONNX Runtime.** *(inferred;
  one of the two things the spike must measure.)*
- **Preprocessing is NOT free**, unlike UForm's. SigLIP wants 256x256 at
  mean/std 0.5, where `image_utils.rs:57,62` has 224 at the CLIP constants.
  Mitigated: `preprocess_moondream_from_image` (`:165-210`) already does 0.5/0.5
  and `preprocess_clip_from_image` is already size-parameterized — so this is
  generalizing one function, not adding a second. *(verified in code.)*
- **A silent-correctness trap.** candle's `TextTransformer::forward` pools at
  `seq_len - 1` (`siglip.rs:730-735`) and `max_position_embeddings` defaults to
  **64**. HF's SigLIP processor pads to `max_length=64`; if the caller tokenizes
  *without* padding to 64, the pooled vector is taken at a different position
  than the checkpoint was trained for and image-text alignment degrades **with
  no error**. Exactly this repo's dominant failure shape. *(verified in vendored
  source; the padding convention is HF-documented behaviour — inferred.)*
- `get_text_features` / `get_image_features` return **un-normalised** features;
  the caller must apply `clip::div_l2_norm` (public, `clip/mod.rs:149`) before
  insert, or every cosine distance is wrong. *(verified in vendored source.)*

### 3.4 Why UForm is a port and SigLIP is not — the distinction revision 1 never drew

UForm also publishes `image_encoder.safetensors` (172 MB) and
`text_encoder.safetensors` (240 MB) *(verified external)*. Revision 1 costed
only the ONNX route and never said why the safetensors route was unavailable.
The answer: **candle-transformers has no UForm implementation** — I listed all
~130 model modules in 0.9.2 and there is none *(verified in vendored source)* —
so safetensors would mean porting the architecture. SigLIP **is** implemented,
which is the whole difference. Stating this in revision 1 would have surfaced
option E.

(Note: UForm's `.onnx` exports are roughly half the size of its `.pt` files, so
they are fp16 — the 222 MB figure is not comparable like-for-like with SigLIP's
1.48 GB f32. *(inferred from the byte counts.)*)

### 3.5 The verdict on the image tower

**Do not pick one on paper.** Both A and E are viable and they trade on
different axes (222 MB fp16 weights plus a release-candidate C++ runtime, versus
1.48 GB f32 weights plus zero new dependencies). Commission **one spike covering
both**, gated on §9's measurement saying an image tower is needed at all:

- **E first**, because it can be tried without adding a dependency: load
  `google/siglip-base-patch16-256-multilingual` through the existing downloader,
  encode 200 images and 20 German + 20 English queries, record recall@10.
- **A second**, and only after the `ort` binary-size spike (1 hour: build a
  hello-world with `ort` on macOS-arm64 and linux-x86_64, diff stripped
  binaries).

A zero-new-dependency candidate must be measured against the same German-query
gate before a release-candidate-only C++ runtime enters the build.

---

## 4. Cost and latency

Absent from revision 1, and asked for explicitly. Figures are for a GPU-less
Hetzner box, 4 vCPU. **All *(inferred)* unless tagged** — arithmetic over
published model sizes and typical CPU throughput, not measured here. They are
order-of-magnitude, and that is enough to order the plan.

| path | download | resident | per image | 50k assets | money |
|---|---|---|---|---|---|
| **CLIP today, discarded** | ~600 MB on first upload | ~350 MB | 50-200 ms | **1-3 CPU-hours of pure waste** | $0 |
| **Tesseract OCR** | ~30 MB per language pack | ~100 MB | 0.2-1 s | **3-14 CPU-hours** | $0 |
| **moondream local caption** | ~1.8 GB | 2-2.5 GB | 10-30 s per prompt; **30-90 s if all three run** | **400-1200 CPU-hours** | $0 |
| **hosted VLM caption** | 0 | 0 | 2-4 s, trivially parallel | wall-clock hours | **~$0.002-0.003/image → ~$100-150 per 50k, ~$2-3k per million** |
| **UForm-ONNX** | 222 MB | ~300 MB | 50-150 ms; ~10-30 ms per text query | 1-2 CPU-hours | $0 |
| **SigLIP-candle** | 1.48 GB | 1.5-2 GB f32 (~750 MB as F16) | 0.5-2 s | 7-28 CPU-hours | $0 |

Four conclusions the table forces:

1. **Deleting the discarded CLIP path (§1.2) is worth more than "hours" implies.**
   It is 1-3 CPU-hours and a 600 MB download per installation that buys nothing.
   Do it in the first commit.
2. **Cost is not the reason to avoid a hosted VLM.** $100-150 for a 50k library
   is noise. Egress and air-gap are the reasons, and they are deployment policy,
   not engineering.
3. **Local captioning has a brutal forward/backward asymmetry.** 30-90 s/image is
   entirely fine for the forward rate of new uploads and **ruinous as a bulk
   backfill** (400-1200 CPU-hours for 50k). Ship moondream for new uploads;
   offer the hosted VLM for the backfill. The plan must name this, or someone
   will start a backfill and the box will be busy for a month.
4. **OCR is two orders of magnitude cheaper than local captioning** and covers
   the half of the corpus where captions paraphrase and image vectors are blind.

**Where the encoder lives at runtime is unmodelled and matters more than the
index memory table.** *(verified in code + inferred.)* `AssetProcessingHandler`
holds `clip_embedder` and `captioner_cache` as single `RwLock<Option<..>>`
fields on the handler — **process-global**. But the tenant embedding config
implies a *per-tenant* model choice, which turns that into an N-model resident
cache on a box that already carries a 512 MB index budget, a ~2.2 GB RocksDB C++
heap and a documented 9.3 GB RSS churn investigation
(`project_rss_churn_investigation`). Two tenants on different caption models
means 5 GB resident, or thrashing. **Decide this explicitly before shipping any
second model:** one model per process (operator-chosen; tenant config *selects
among installed* models), or an LRU with a declared budget. For a multi-tenant
deployment this is a bigger production risk than anything in §5.5.

---

## 5. The HNSW prerequisite — real, but re-priced

Revision 1 put this first and sized it at 3-4 weeks. The analysis survived both
reviews intact; **the sequencing did not.** The section stands; §5.6 re-prices
it.

### 5.1 The problem *(verified in code)*

```rust
// crates/raisin-hnsw/src/engine/mod.rs:254-256
fn make_key(&self, tenant_id: &str, repo_id: &str, branch: &str) -> String {
    format!("{}/{}/{}", tenant_id, repo_id, branch)
}
```

The storage key knows about embedders and kinds:

```
{tenant}\0{repo}\0{branch}\0{workspace}\0{embedder_hash:11}\0{kind:1}\0{source_id}\0{chunk_idx:04}\0{revision}
                                          ^^^^^^^^^^^^^^^^^  ^^^^^^^^
                                          which model         T or I
```

(`raisin-embeddings/src/models.rs:18`.) The index does not. The single-space
assumption is **four layers deep**:

1. the index key above;
2. `dimensions_for(&self, tenant_id, _repo_id, _branch)` — one width per tenant,
   with the comment *"Config is keyed by tenant alone today"*
   (`raisin-rocksdb/src/repositories/tenant_embedding_config.rs:150-167`);
3. the distance metric — **and this layer has already drifted, see §5.2**;
4. the query embedder — one process-wide per-tenant installed resolver.

### 5.2 Correction: the metric layer is not as simple as revision 1 said *(verified in code)*

Revision 1 wrote that the metric is "one field on the whole engine, always
`DistanceMetric::default()`". That is true of the **engine**
(`engine/mod.rs:62`; `new()` at `:93-101` passes `DistanceMetric::default()`,
and `startup/indexing.rs:51-56` calls `new`) — but it misses a live drift:

- a **per-tenant** `distance_metric` exists and is settable: rendered by `SHOW`
  (`raisin-sql-execution/src/engine/ai_config.rs:75`), parsed by
  `ALTER EMBEDDING CONFIG` (`:178`, via `parse_distance_metric` at `:706`);
- and it is consumed **at query time**
  (`scan_executors/vector_scan.rs:109`:
  `distance_metric.to_hnsw_metric().requires_normalization()`),
- while the index is always **built** with the engine's default.

**So a tenant can already configure a query metric that disagrees with the
metric its index was constructed under.** This matters beyond tidiness: §5.5's
i8 argument leans on "the metric is Cosine and every vector is normalised," and
the metric is configurable, not fixed. `IndexSpecResolver` (§5.4) is where the
two must be reconciled — the metric an index was *built* with belongs in its
sidecar and must be what the query uses.

### 5.3 What actually breaks: availability, not garbage *(verified in code)*

Everything fails closed:

- **insert** of a wrong-width vector returns a clean `Err` before touching
  usearch (`index.rs:265-271`) — but `store_embedding` already ran, so the row is
  durable and the job retries forever: embeddings count climbs, index count does
  not;
- **load** with a width disagreement returns a loud error naming
  `REBUILD VECTOR INDEX` (`engine/mod.rs:199-235`); an *empty* index at the wrong
  width is silently adopted at the configured width, which is correct;
- **query** width-checks, and usearch itself throws underneath.

So the real blocker is **mutual exclusion**: one index per branch, every entry
point through `get_or_load_index`, therefore **turning on image embedding takes
text search down.**

**The one genuinely silent case** is two embedders of the *same* width. Nothing
in `raisin-hnsw` compares anything but `dimensions`; `IndexMetadata` and
`NodeMeta` carry no embedder identity. Two same-width models occupy unrelated
regions of R^n; every distance is finite, every ranking plausible, nothing logs.
Width checks can never catch it — only partitioning can.

### 5.4 The design (unchanged, and still right)

```
{tenant}/{repo}/{branch}/{embedder_hash}{kind}
  →  base/<tenant>/<repo>/<branch>/<hash>T.hnsw  +  .hnsw.meta
```

- **Both segments.** `kind` alone cannot separate two text models;
  `embedder_hash` alone is numerically sufficient, but `kind` is one character,
  already in the CF key, and lets a reader address "the text partitions" without
  decoding a hash.
- **Crate-boundary constraint** *(verified in code)*: `raisin-hnsw`'s only raisin
  deps are `raisin-error` and `raisin-hlc` (`Cargo.toml`), so it cannot see
  `EmbedderId` / `EmbeddingKind` (those live in `raisin-ai::config`, which pulls
  candle and tesseract). The engine therefore takes an opaque
  `PartitionId(String)` newtype, and the **single** rendering lives beside
  `EmbedderId` as `EmbeddingPartition::to_index_token()`, **with a unit test
  asserting its bytes equal segments 5 and 6 of the CF key.** One derivation, two
  renderings, one test that fails on drift. A newtype rather than `&str` because
  `add_embedding` would otherwise take three same-typed strings.
- **Generalise the resolver, do not add a sibling.**
  `dimensions_for(t,r,b) -> Option<usize>` becomes
  `spec_for(t,r,b,&PartitionId) -> Option<IndexSpec>` with
  `IndexSpec { dimensions, metric, quantization, params }` — and this is where
  §5.2's metric drift gets reconciled. A parallel `QuantizationResolver` beside
  `dims_resolver` is exactly the mirrored-path shape this codebase keeps losing.
- **Change the existing signatures; do not add partition-aware variants.** Eleven
  engine methods plus call sites; `copy_for_branch` becomes a per-partition copy.
  Mechanical; the compiler finds them all.

Two pre-existing defects this fixes or exposes *(both verified in code)*:

- `get_index_path` (`engine/mod.rs:258-264`) pushes each `/`-split segment then
  calls `.with_extension("hnsw")` on the last — so branch `release.2` and
  `release.3` **both write `release.hnsw`**. Making the branch a directory
  component removes it. (The `embedder_hash` alphabet is `URL_SAFE_NO_PAD`
  base64, no dots, so the token is a safe file stem.)
- `hnsw_transfer/manager.rs:32-37` builds
  `index_base_dir.join(tenant).join(repo).join(format!("{}.hnsw", branch))`
  **independently** — already drifted for those branch names — and a grep for
  "meta" across the whole module returns only `fs::metadata`: **the `.hnsw.meta`
  sidecar is never shipped.** A peer receiving a bare `.hnsw` routes into
  `migration::migrate_from_old_format`, which bincode-deserialises a usearch file
  and fails. The index is gone. This is a second implementation of the index path
  and must be folded into the engine's one builder **before** partitioning
  multiplies it.

**Migration is a rename, not a rebuild.** Because the dims resolver is per-tenant
and ignores repo and branch, **every existing index has exactly one possible
partition**: the tenant's resolved embedder, kind `Text`. On load, if
`base/t/r/branch.hnsw` exists and `base/t/r/branch/{token}.hnsw` does not, move
both files. Zero vectors re-encoded, idempotent. Guard with the width check that
already exists. **Do not route it through `migration.rs`** — that is a *format*
migration triggered by a missing sidecar, and feeding a v2 index into it is
precisely the `hnsw_transfer` bug. Put it in `persistence.rs` beside
`meta_path_for`.

**Doing this after image vectors exist costs a full `REBUILD VECTOR INDEX` on
every branch**, because an index containing two spaces cannot be split by
rename — the vectors are interleaved in one graph and nothing on disk records
which is which. That asymmetry is real. **But note what it constrains:
partitioning must precede the FIRST IMAGE VECTOR, not today.** That is the
re-pricing in §5.6.

### 5.5 Quantization: a shipped, configurable, inert UI *(corrected)*

Revision 1 said a repo-wide grep for `QuantizationType|HnswParams` outside
`raisin-hnsw` "returns nothing (verified)." **That is false, and the truth makes
the case stronger while changing the work item.** *(verified in code:)*

- `packages/admin-console/src/api/ai.ts:40-50` **defines**
  `QuantizationType = 'F32'|'F16'|'Int8'` and `HnswParams`, and `:87-88` hangs
  both off `EmbeddingSettings`;
- `TenantAiSettings.tsx:1234-1237` and
  `TenantEmbeddingSettings.tsx:132,246,749` render a live `<select>` and **send
  `quantization` in the save payload**;
- the Rust side is empty:
  `grep -rn quantization crates/ --exclude-dir=.admin-console-dist` outside
  `raisin-hnsw` returns exactly one hit, an unrelated `quantization_level` in
  `providers/ollama/api_types.rs:121`.

So an operator can select **Int8** in the console today, the setting is dropped
on the way in, and **every index is F32 regardless**. This is the same
configurable-but-inert trap as the captioning controls in §1.3 — the one this
document exists to stop. **Item 0d is therefore not "wire I8"; it is "wire I8
*and* reconcile a UI that already promises it,"** and it carries the same
it-looks-like-it-works risk.

The plumbing inside `raisin-hnsw` is genuinely complete: `QuantizationType` maps
to `usearch::ScalarKind`, is persisted in the sidecar with `#[serde(default)]`
and restored on load; `with_params` is called from one place with
`HnswParams::default()`. F16 and I8 are a **config change only** — usearch casts
the caller's `&[f32]` into the index's scalar kind on insert and
`scalar_words() == dimensions` for those kinds, so add/search signatures are
untouched. The f32→i8 cast L2-normalises then scales to ±127 and assumes a
dot-product-like metric — a precondition this codebase *usually* satisfies, but
see §5.2: the metric is configurable, so `IndexSpecResolver` must pin it.

**B1/Hamming is not a config change** and is deferred with a named reason:
`scalar_words()` for b1 is `dimensions/8`, so an f32 slice is rejected at the
binding boundary. It needs `add_b1x8` / `search_b1x8` / `filtered_search_b1x8`,
caller-side bit packing, and a distance scale where `DEFAULT_MAX_DISTANCE = 0.6`
means something. `QuantizationType` has no `B1` variant.

**Memory per 50k vectors**, against the **512 MB engine budget shared by all
tenants** (`startup/indexing.rs:57` — `let cache_size = 512 * 1024 * 1024;`,
*verified in code*). Payload arithmetic is exact; the graph+sidecar floor is
usearch's documented ~200 B/vec at M=16 plus this crate's own 144 B/vec
(`index.rs:690-698`: `len*64 + len*80`) — *(inferred, not measured)*:

| partition | payload | +floor | total | % of 512 MB |
|---|---|---|---|---|
| bge-m3 text, 1024 f32 | 204.8 MB | 17.2 | **222 MB** | 43% |
| bge-m3 text, 1024 **i8** | 51.2 MB | 17.2 | **68 MB** | 13% |
| **SigLIP image, 768 f32** | 153.6 MB | 17.2 | **171 MB** | 33% |
| **SigLIP image, 768 i8** | 38.4 MB | 17.2 | **56 MB** | 11% |
| CLIP 512 f32 | 102.4 MB | 17.2 | **120 MB** | 23% |
| UForm 256 f32 | 51.2 MB | 17.2 | **68 MB** | 13% |
| UForm 256 **i8** | 12.8 MB | 17.2 | **30 MB** | 6% |
| UForm 64 f32 (Matryoshka) | 12.8 MB | 17.2 | **30 MB** | 6% |

Three readings:

1. there is a **~17 MB floor per 50k vectors regardless of width**, so
   Matryoshka's 16x payload cut becomes a 2.3x total cut;
2. **256-dim i8 and 64-dim f32 both cost 30 MB**, and i8 is the better of the
   two — it keeps all 256 dimensions at 8-bit precision instead of discarding
   192. Matryoshka is real but **i8 is the lever to reach for first**;
3. **the biggest single win is on the text index.** Flipping the existing
   1024-dim bge-m3 index to i8 saves 154 MB — **more than any image partition
   costs** — with no new model, no new dependency, and the enum arm already
   written. This is why 0d is independently justified today.

SigLIP's row is the honest counterweight to §3.3: at 768 dims it costs ~2.5x
UForm's index memory (171 MB vs 68 MB f32), which partly offsets its
zero-dependency advantage. **A real trade, not a strict win.**

**The weigher caveat, mechanism corrected.** Revision 1 said moka "inserts a
freshly created *empty* index," implying every weight is ~0. **That is wrong and
would send an implementer to the wrong line.** *(verified in code:)* in
`get_or_load_index` the index is loaded from disk via
`HnswIndex::view_from_file(&path)` **before** `index_cache.insert(key, ...)`
(`engine/mod.rs:~200-247`), so a loaded index **is** weighed with its real
content. The actual defect is that **moka never re-weighs after insert**: a
newly created index is pinned at ~0 forever, and a loaded one is pinned at its
load-time size while it grows. Same conclusion — **the 512 MB budget bounds
nothing** and §5.5's numbers are unobservable until it is fixed — but the fix is
"re-insert on growth, or weigh a maintained counter," not "weigh at load."

A related hazard that partition count makes worse: an **evicted dirty index is
dropped from the dirty set without being saved** (`engine/lifecycle.rs:70-74` —
the `else` branch commented *"Index was evicted, remove from dirty set"*),
silently losing unsaved vectors. More cache entries means more eviction
pressure. Another reason 0b precedes 0c. *(verified in code.)*

### 5.6 Re-pricing: split Phase 0

Revision 1 bundled four items into "3-4 weeks of prerequisite" and put it in
front of everything. That framing is what makes the cheap-looking Phase 1 the
only affordable move. Unbundled:

| item | independently justified **today**? | why |
|---|---|---|
| **0a** fold `hnsw_transfer` into the one path builder, ship the `.hnsw.meta` | **YES** | a live replication bug that destroys a transferred index |
| **0b** fix the moka weigher | **YES** | the 512 MB budget currently bounds nothing |
| **0d** `IndexSpecResolver`, wire I8, reconcile the console UI | **YES** | 222 MB → 68 MB on the *existing* text index; and a shipped control currently lies |
| **0c** partition the index by embedder+kind | **NO** | buys nothing until a second space exists |

0a + 0b + 0d are about a week combined and each ships a visible win. 0c is
1.5-2 weeks — two-thirds of the bill — and its rename-vs-rebuild asymmetry
constrains it relative to the **first image vector**, not relative to today.
**Buy 0c when the image tower is commissioned.** If §9's measurement says
OCR + captions are enough, 0c may never be needed.

### 5.7 Relationship to the `workspaces =>` scoping that just landed

**Complement, not alternative.** They partition orthogonal dimensions for
opposite reasons:

- **workspace = a predicate inside one index.** `search_scoped` /
  `filtered_matches` filter inside the graph walk; out-of-scope candidates are
  still expanded *through*, so the graph stays connected. That works only because
  every vector is in the same space.
- **embedder/kind = separate indexes.** A 256-dim image vector is not a *worse*
  neighbour of a 1024-dim text query; it is not a neighbour at all, and usearch
  rejects the query. **You cannot make a mixed-width index navigable with a
  predicate.**

They compose in the useful direction: `filtered_search_i8` and
`filtered_search_b1x8` both exist in usearch 2.24
(`MEASURED-usearch-filtered-search.md`), so the scoped walk works over a
quantized image partition.

One coupling to handle deliberately: `ScopeFilterMode` gains a third cause of
"zero results" — *queried the wrong partition* — indistinguishable from
`IndexSide` with `in_scope_total == 0`. **Put the partition in `ScopedSearch`
and in the `fetch_scoped` log lines, and print it per leg in `EXPLAIN`.**
Revision 1 stated this in prose and never gave it a line item, so it would have
been the first thing cut; it is now item 4g in §8.

---

## 6. Correctness rules that must not be lost

### 6.1 Both instruments on one node, and the delete side must be widened *(verified in code)*

The CF key already separates them, so no new store is needed. For one asset
`/assets/harbour.jpg`:

| row | embedder_hash | kind | content |
|---|---|---|---|
| 1 | `<bge-m3>` | `T` | OCR text + caption + alt_text + keywords, chunk 0 |
| 2 | `<bge-m3>` | `T` | chunk 1, if long enough |
| 3 | `<image model>` | `I` | the image vector |

**The delete side is a live hazard the moment kind `I` exists.**
`jobs/handlers/embedding/handler.rs:243` computes
`let kind_char = raisin_ai::config::EmbeddingKind::Text.to_key_char();` **once**
and passes that same char to both `is_already_current` (`:267`) **and**
`remove_old_chunks_from_hnsw` (`:284`). Adding an Image kind without widening
the tombstone prefix leaves image vectors that survive every update and every
delete — **the exact shape of the spatial-index bug recorded in CLAUDE.md.**

### 6.2 Fuse ranks, never distances

**Distances from different vector spaces are not comparable.** A cosine 0.31
against bge-m3 and 0.31 against SigLIP are not the same evidence, and no
normalisation makes them so. The existing fusion is already rank-based
(`search/fusion.rs:45-85`: `score = w_ft/(RRF_K + rank_ft) + w_vec/(RRF_K +
rank_vec)`, `RRF_K = 60.0`, total-order tie-break) — but it is hard-wired to two
legs in three places: the `fuse()` signature, `FusedHit`'s named
`fulltext_rank` / `vector_rank` fields, and the emitted columns. *(verified in
code.)* A third leg means:

- generalise to `fuse(legs: &[(f64, &RankMap)])` with `ranks: Vec<Option<usize>>`
  and a leg-name table. `RRF_K` and `sort_total` are untouched — RRF is defined
  over any number of rankers, and the existing
  `default_weights_reproduce_plain_rrf` test (`:111`) still pins the two-leg
  arithmetic;
- **keep the existing column names.** Add `image_rank` beside `fulltext_rank` and
  `vector_rank`; do **not** rename to `rank_0..n`. Agents' SQL depends on them;
- `image_weight => 0` by default, and zero skips the leg *including provider
  resolution*.

**This only matters for Stage 4.** OCR and captions land in the *existing* text
partition and need no fusion change at all — which is a large part of why they
go first.

### 6.3 The image leg's query vector must come from the image model's own text tower

**The single most dangerous thing to get wrong, because it fails silently and
plausibly.** An image-text model's text tower and bge-m3's text tower are
different spaces. Fusing "query → bge-m3 → text index" with "query → bge-m3 →
image index" is nonsense: the image leg returns noise and RRF launders it into
the results under a plausible `image_rank`. That is precisely the hazard
`raisin-embeddings/src/resolve.rs:87-101` documents ("confident nonsense, no
error anywhere"). It falls out of §5.4 for free if the query embedder is
partition-tagged, and not at all if it isn't.

### 6.4 Derived state replicates; side effects do not

**An image embedding is derived state and must run on every replica; a hosted
caption is a side effect and must run once.** They sit on opposite sides of the
`!is_remote_event` gate even though they concern the same node. A caption lands
as a property, replicates as a record, and the arriving replica embeds it locally
through the ordinary funnel. Getting this backwards produces a replica with
captions and no image vectors — which, because `HYBRID_SEARCH` fuses legs, does
not error; it silently returns fewer results. **That failure has already happened
once in this codebase, to the text embedding.**

Same rule for OCR: it is deterministic derived state, so it can run per-replica
or replicate as a property. Prefer replicating the property — one Tesseract run
is cheaper than N.

---

## 7. Pipeline fit

### 7.1 Where each piece runs

| piece | half | why |
|---|---|---|
| **image OCR** | native (`AssetProcessing` job) | needs raw `BinaryStorage` bytes, must survive a restart, must be pool-rate-limited, and an uploaded JPEG is attacker-controlled input to a decoder. Extends `is_extractable_mime` / `process_extractable` — **one vocabulary, one dispatch point** (§1.5) |
| **captioning (local)** | native, same job | the implementation is already there and already loaded from that handler (§1.3) |
| **captioning (hosted VLM)** | delegated (Studio trigger fn) | a network side effect with a per-call cost; must not run on every replica |
| **image embedding** | native, same job | derived state; finishes the existing CLIP site rather than adding a third (§1.4) |

**It must not need the media plugin, and it does not.** *(verified in code:)*
`image_embedding`, `image_caption` and `image_keywords` are all
`TaskProvider::Native` (`rules/tasks.rs:76-95`), and `plan_tasks(..., |_| false)`
keeps native tasks runnable (`:266-269`). `image_ocr` is `Plugin` **only by an
inconsistent classification** (§1.5) — `extract_text`, which uses the same OCR
provider, is `Native`. Decode is in-process: `image` 0.25 is already a dependency
and `preprocess_clip_from_image` already does resize-to-fill.

**Do not "improve" this by routing image work through the delegated half to
reuse a plugin resize.** That would make a core search capability depend on an
optional binary and would put a vector write above `raisin-functions`, where no
binding can store one.

### 7.2 Thumbnails stay orthogonal

An encoder does its own resize from the **original** bytes; it must not consume a
thumbnail derivative, because a thumbnail's crop and compression are tuned for a
human eye and would silently change every vector if someone re-tuned them. Two
consumers of one derived artifact is how a UI change becomes a retrieval
regression with no error anywhere. Thumbnails remain unbuilt and remain the
delegated half's business.

### 7.3 Three couplings at the node-local artifact

The pipeline's spine is right and this work joins it without change: **the two
halves meet at a node property.** OCR writes `extracted_text`; captioning writes
`description` / `alt_text` / `keywords`; the write emits `node:updated`; the
existing funnel does fulltext + embedding.

- **`extraction_fingerprint` is the loop-breaker.** `should_process_asset` gates
  on it, so a caption or OCR write that changes the fingerprint re-enqueues asset
  processing **forever**. Both must be part of the same fingerprinted unit, or
  explicitly excluded from it. *(verified in code — the property exists with
  `index: [Property]` at `raisin_asset.yaml:118-121`.)*
- **Per-rule chunking already reaches the embedder.** A caption is one chunk;
  OCR of a dense slide may be several. Both already handled.
- **Content-hash dedup is not wired on the upload path**, so a re-upload of
  identical bytes re-extracts, re-captions and re-embeds. Revision 1 flagged this
  and then never estimated it. **With a paid VLM it is the difference between a
  bounded and an unbounded bill.** It is now item 1f. A `content_hash` property
  already exists with `index: [Property]` (`raisin_asset.yaml:80-83`), so the
  lookup is a property-index probe, not new infrastructure. *(verified in code.)*

---

## 8. The plan

Estimates in two units, because the first is misleading here. **Agent-sessions**
is the unit this repo actually ships in — the working tree this design was
written in carries 144 uncommitted files of hybrid-search and workspace-scoping
work produced inside a single workflow. **Human-weeks** is given for
cross-reading against outside expectations. Both are engineering estimates, not
measurements; the ratios are more reliable than the absolutes.

### Stage 1 — ship search over images (do all of it)

| # | item | sessions | weeks |
|---|---|---|---|
| **1a** | **image OCR → `extracted_text`.** Reclassify `image_ocr` to `Native` (`tasks.rs:133`); widen `is_extractable_mime` and add the branch to `process_extractable` (`helpers.rs:224-241`); call `ocr_image` in the asset job; **add an explicit `ocr` feature to `raisin-rocksdb`**; **fix `de`→`deu` mapping and make the language configurable per tenant** | 1-2 | 3-5 d |
| **1b** | **wire the existing local captioner.** Call `generate_image_caption` / `generate_image_keywords` from `handle` (`handler.rs:219-259`, already implemented); write `description` / `alt_text` / `keywords`; respect `extraction_fingerprint`; delete the `tracing::warn!` at `:567-585` | 1 | 2-4 d |
| **1c** | **hosted-VLM caption path** as the alternative to 1b for quality and for backfill throughput: a Studio trigger fn calling `raisin.ai.completion()` with `ContentPart::image`. Caption **in the tenant's language** | 1-2 | 3-5 d |
| **1d** | **delete `process_image_embedding`** (or make its dead end loud). Reclaims 1-3 CPU-hours per 50k and a 600 MB download per installation (§4) | <1 | hours |
| **1e** | **the YAML change**: `index: [Fulltext, Vector]` on `description` / `alt_text` / `keywords`, bump `version:` | <1 | hours |
| **1f** | **content-hash dedup on the upload path** (§7.3). Bounds the VLM bill | 1 | 2-3 d |
| **1g** | **the backfill**, which revision 1 hid inside 1e — see the warning below | 1 | 2-3 d |
| **1h** | reconcile the console: `caption_model` and the three prompts now reach a real implementation; **`quantization` still does not** (§5.5) — say so in the UI, or wire it in 0d | <1 | 1-2 d |

**Stage 1 total: 6-9 sessions / about 2-3 weeks.**

> **The backfill is a real cost revision 1 did not price.** Adding `Vector` to
> three properties changes retrieval **only for assets written after the
> change**. `REBUILD VECTOR INDEX` rebuilds the *graph* from stored vectors
> (`engine/ai_config.rs:453-499` → `HnswManagement::rebuild_index`); it does not
> regenerate them. The backfill is the `force` path in
> `management/database/vector_embeddings.rs:283-345`, which iterates nodes that
> **already hold an embedding row** and re-queues `EmbeddingGenerate` carrying
> `FORCE_REEMBED_KEY`. *(verified in code.)* That mechanism exists — good — but
> it is **tenant-wide**, costs one provider call per node, and will stampede the
> job queue exactly when the release lands. It needs a repo/workspace scope and a
> rate limit. (Every asset does get an embedding row today, because name+path are
> included per tenant config, so the sweep does reach image assets.)
>
> **And `alt_text` currently has no `index:` key at all**, so writing
> `[Fulltext, Vector]` there makes it **fulltext-visible for the first time**.
> Probably desirable — but it is a second, undeclared change hidden in a one-line
> diff, and it will move existing fulltext rankings.

### Stage 2 — the infrastructure that pays for itself today

| # | item | sessions | weeks |
|---|---|---|---|
| **0a** | fold `hnsw_transfer`'s path building into the engine's one builder; ship the `.hnsw.meta`; regression test that a transferred index loads | 1 | 2-3 d |
| **0b** | fix the moka weigher so resident index memory is measured (re-weigh on growth) | 1 | 1-2 d |
| **0d** | `IndexSpecResolver` replacing `EmbeddingDimsResolver`; **pin the build-time metric in the sidecar and reconcile §5.2's drift**; wire I8; make the console's `quantization` control real | 1-2 | 3-5 d |

**Stage 2 total: 3-4 sessions / about 1 week.** Each item is independently
justified: 0a is a live replication bug, 0b makes a budget real, 0d is
222 MB → 68 MB on the existing text index plus a UI that stops lying.

### Stage 3 — measure (the gate; see §9)

| # | item | sessions | weeks |
|---|---|---|---|
| **3a** | **build the labelled corpus first**: ~200 real Studio assets spanning both halves, ~30 German and ~30 English queries, relevance judgments | 1-2 | 3-5 d |
| **3b** | run **three arms on that one corpus**: OCR-only, caption-only, OCR+caption. Record recall@10 per arm, per language, per corpus half | 1 | 2-3 d |

**Stage 3 total: 2-3 sessions / about 1 week.** This is the decision point.

### Stage 4 — an image tower, ONLY if Stage 3 says so

| # | item | sessions | weeks |
|---|---|---|---|
| **4a** | **spike E (SigLIP-in-candle) first** — zero new deps: load the multilingual checkpoint through the existing downloader, generalise `preprocess_*` for 256 @ 0.5/0.5, pad to 64 tokens, `div_l2_norm`, run 3b's corpus as a fourth arm | 1-2 | 3-5 d |
| **4b** | **spike A (`ort`) binary size** on macOS-arm64 and linux-x86_64: hello-world, diff stripped binaries | <1 | 1 h |
| **4c** | **0c: partition the index** — `PartitionId` newtype, `to_index_token()` + drift test, eleven engine signatures, lazy rename, per-partition `SHOW` / `VERIFY` / `REBUILD` | 2-3 | 1.5-2 w |
| **4d** | construct `EmbeddingKind::Image`, write through the ONE embedding path, **widen the delete/tombstone side for the new kind** (§6.1) | 1-2 | 3-5 d |
| **4e** | partition-tagged query embedder so the image leg uses its own text tower (§6.3) | 1 | 2-3 d |
| **4f** | third RRF leg: generalise `fuse`, `image_rank` column, `image_weight => 0` default, `image =>` query-by-example | 1-2 | 1 w |
| **4g** | **observability**: partition in `ScopedSearch`, in the `fetch_scoped` log lines, and per leg in `EXPLAIN` (§5.7) | <1 | 1-2 d |

**Stage 4 total: 7-11 sessions / 3-4 weeks.** 4c is the item whose price rises
after the first image vector; everything before it is reversible.

### The blunt version

Stage 1 + Stage 2 is roughly **9-13 sessions / 3-4 weeks**, every item
independently justified, and it ships: image search that works in German, real
`alt` attributes, a 154 MB memory win on the existing text index, and a fixed
replication bug. Stage 3 then decides whether Stage 4 is worth 3-4 more weeks.

---

## 9. The measurement gate — three arms on one corpus

Revision 1 made "caption a representative corpus, run German and English
queries, record recall@10 — 2-3 days" the gate on everything, with **no corpus,
no relevance judgments and no baseline**. One arm measured against nothing is
not a gate; it is a vibe.

Build the corpus **first** (item 3a), then measure **three arms in one
afternoon**:

| arm | what is indexed | what it tests |
|---|---|---|
| **OCR-only** | `extracted_text` from Tesseract | the text-dominant half. The cheapest possible baseline |
| **caption-only** | `description` + `alt_text` + `keywords` | the photographic half, and cross-lingual inheritance |
| **OCR + caption** | both | whether they compose or interfere |
| *(later)* **+ image tower** | an image partition too | whether the residual justifies Stage 4 |

Report **recall@10 split by corpus half and by query language**, against a
same-corpus fulltext-only baseline. The decision rule, fixed in advance:

- if OCR+caption clears the bar on both halves in German → **Stage 4 is not
  bought**;
- if the photographic half is weak in German → caption *in* German first,
  re-measure, and only then consider an image tower;
- if real queries are dominated by "the image that looks like this one",
  near-duplicate detection, or "the slide with this chart" → **no caption of any
  length answers them** and Stage 4 is justified on merit.

---

## 10. What would change this recommendation

- **Stage 3 shows OCR and captions are enough.** Then Stage 4 and 0c are never
  bought, and this document ends at three to four weeks.
- **The real corpus is dominated by query-by-example / near-duplicate work.**
  Then Stage 4 moves up and 0c with it.
- **SigLIP's 1.48 GB footprint is unacceptable on the target box.** Then A
  (UForm, 222 MB) becomes the image tower, and 4b's binary-size number becomes
  load-bearing.
- **`ort` reaches a stable 2.0** with a macOS-arm64 prebuilt. That removes most
  of A's non-0c cost and probably makes A beat E.
- **Egress becomes acceptable.** Then Cohere Embed v4 (option D) is the best
  image tower on merit — 100+ languages, native Matryoshka, a `VoyageProvider`
  sibling already at `provider.rs:258` to copy, zero binary, zero weights — and
  its air-gap failure mode is identical to a hosted VLM the deployment already
  accepted.
- **Someone commits to multi-space HNSW for another reason** (multi-model text,
  say). Then 0c is already paid and Stage 4 should be re-evaluated the same day.

## 11. What is not verified here

- **The corpus split ratio.** The text-dominant / photographic division in §2 is
  a claim about brand-asset libraries in general, not a measurement of a real
  Studio tenant. Item 3a settles it, and it is the input that most changes the
  answer.
- **Every number in §4** except the download sizes. Arithmetic over published
  model sizes and typical CPU throughput, not measurements on the target
  hardware.
- **ONNX Runtime's static-link binary cost.** Mechanism understood, megabytes
  unknown. Item 4b.
- **`ort`'s current macOS-arm64 prebuilt coverage.** The failures cited are from
  issue trackers on older release candidates, not a build attempt here.
- **SigLIP's German retrieval quality.** "No language filter on WebLI" is a
  training-data claim, not a recall number. Item 4a.
- **UForm's retrieval quality on a European corpus.** 21 languages is a coverage
  claim. A 206M model spreading 21 languages across a small text tower will
  plausibly not match a 568M bge-m3 trained specifically for multilingual
  retrieval — *(inferred)*, and no vendor publishes a per-language recall number
  for it.
- **CLIP-family weakness on rendered text.** Well documented in the literature
  and consistent with 16px patches at 224-256px, but not measured here. It is
  load-bearing for §2's conclusion, and item 3b measures it directly.
- **The per-vector memory figures in §5.5.** Payload arithmetic is exact; the
  ~17 MB floor is usearch's documented layout plus this crate's own
  `estimated_memory_bytes` formula — which does **not** count the two `String`
  allocations per entry, so real heap is somewhat higher. Item 0b makes them
  observable.
- **Cohere Embed v4's behaviour**, vendor-doc sourced only.
