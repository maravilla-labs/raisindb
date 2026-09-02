//! Embedding job handler struct and methods.
//!
//! Contains the `EmbeddingJobHandler` struct with methods for handling
//! embedding generation, deletion, and branch copy operations.

use super::content_extraction::{extract_embeddable_content, hash_text};
use super::upstream::upstream_key;
use crate::jobs::circuit_breaker::CircuitBreakerRegistry;
use crate::jobs::handlers::fulltext::IndexPlanCache;
use crate::RocksDBStorage;
use raisin_embeddings::config::TenantEmbeddingConfig;
use raisin_embeddings::models::EmbeddingData;
use raisin_embeddings::resolve::ResolvedEmbeddingProvider;
use raisin_embeddings::EmbeddingStorage;
use raisin_embeddings::TenantEmbeddingConfigStore;
use raisin_error::{Error, Result};
use raisin_hnsw::HnswIndexingEngine;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::sync::Arc;

/// The name of the spec that embeds a node's EXTRACTION ARTIFACT.
///
/// Written down once, here, and validated by a test against
/// `raisin_hnsw::is_valid_spec_name` — a name that fails that grammar produces
/// index ids the parse cannot take apart, and therefore `SearchResult`s whose
/// `node_id` cannot be fetched.
pub const EXTRACTED_TEXT_SPEC: &str = "doc";

/// One named embedding a node should carry, and the text it is made from.
struct EmbedTask {
    /// `None` is the default spec — the node's own authored fields, filed under
    /// the bare node id exactly as before named specs existed.
    spec: Option<&'static str>,
    /// The text to embed. Never empty: a spec with no text is not produced at
    /// all, because a vector of the empty string is a near-neighbour of every
    /// query.
    text: String,
}

/// Handler for embedding generation jobs
///
/// This handler processes embedding-related jobs by:
/// 1. Fetching tenant embedding configuration
/// 2. Decrypting API keys
/// 3. Creating embedding providers
/// 4. Generating or managing embeddings
/// 5. Updating HNSW index
pub struct EmbeddingJobHandler {
    storage: Arc<RocksDBStorage>,
    hnsw_engine: Arc<HnswIndexingEngine>,
    master_key: [u8; 32],
    /// Per-type vector indexing plan cache, shared across nodes for the handler's
    /// lifetime (resolved for `IndexType::Vector`; never collides with fulltext).
    plan_cache: Arc<IndexPlanCache>,
    /// Upstream circuit breakers.
    ///
    /// Defaults to the PROCESS-WIDE registry, because the sharing is the whole
    /// feature: a per-handler breaker would mean each population of jobs
    /// rediscovering the same outage. Only tests substitute their own, via
    /// [`EmbeddingJobHandler::with_breakers`].
    breakers: Arc<CircuitBreakerRegistry>,
}

impl EmbeddingJobHandler {
    /// Create a new embedding job handler
    pub fn new(
        storage: Arc<RocksDBStorage>,
        hnsw_engine: Arc<HnswIndexingEngine>,
        master_key: [u8; 32],
    ) -> Self {
        Self {
            storage,
            hnsw_engine,
            master_key,
            plan_cache: Arc::new(IndexPlanCache::new()),
            breakers: Arc::clone(CircuitBreakerRegistry::global()),
        }
    }

    /// Same handler, against a caller-supplied breaker registry.
    ///
    /// For tests, which must neither observe nor pollute the process-wide
    /// registry — a breaker opened by one test would park the next one.
    pub fn with_breakers(mut self, breakers: Arc<CircuitBreakerRegistry>) -> Self {
        self.breakers = breakers;
        self
    }

    /// The breakers this handler consults, for an admin surface to read.
    pub fn breakers(&self) -> &Arc<CircuitBreakerRegistry> {
        &self.breakers
    }

    /// Handle embedding generation job
    ///
    /// This method:
    /// 1. Extracts node_id from JobType::EmbeddingGenerate
    /// 2. Gets tenant embedding config from storage
    /// 3. Decrypts API key using ApiKeyEncryptor
    /// 4. Creates embedding provider
    /// 5. Fetches node and extracts embeddable content
    /// 6. Generates embedding via provider
    /// 7. Stores in RocksDB embedding storage
    /// 8. Adds to HNSW index
    pub async fn handle_generate(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract node_id from job type
        let node_id = match &job.job_type {
            JobType::EmbeddingGenerate { node_id } => node_id,
            _ => {
                return Err(Error::Validation(format!(
                    "Expected EmbeddingGenerate job, got {}",
                    job.job_type
                )))
            }
        };

        tracing::debug!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            workspace_id = %context.workspace_id,
            node_id = %node_id,
            revision = %context.revision,
            "Processing embedding generation job"
        );

        // Get tenant embedding config
        let config_repo = self.storage.tenant_embedding_config_repository();
        let config = config_repo
            .get_config(&context.tenant_id)
            .map_err(|e| Error::storage(e.to_string()))?
            .ok_or_else(|| Error::NotFound("No embedding config for tenant".to_string()))?;

        if !config.enabled {
            tracing::debug!(
                tenant_id = %context.tenant_id,
                "Embeddings disabled for tenant, skipping"
            );
            return Ok(());
        }

        // Resolve provider - use unified AI config if available, otherwise legacy fields
        let resolved = self
            .resolve_embedding_provider(&config, &context.tenant_id)
            .await?;

        // ── THE BREAKER GATE ────────────────────────────────────────────────
        //
        // Everything below this line is expensive: fetching the node, resolving
        // its index plan, reading the extraction artifact, resolving the
        // chunking rules, and then TOKENIZING AND CHUNKING the text — which for
        // a 40-page document is the bulk of a job's CPU. In the 2026-09-02
        // incident all of that ran 2,512 times against an upstream that was
        // returning 503, because the cheap failure happened LAST.
        //
        // So the gate is here, as early as it can possibly be: the resolution
        // above is the first point at which the upstream's identity is known,
        // and it costs a config read and one decrypt. It deliberately sits
        // BEFORE `resolved.build()` too — the client is cheap to construct, but
        // "before the expensive work" is a property that is easier to keep if
        // nothing at all happens between knowing the key and asking.
        //
        // An `Err` here is `Error::UpstreamUnavailable`, which the job worker
        // PARKS on rather than retries: nothing was attempted, so nothing
        // failed, and burning a retry on an attempt that never happened is how
        // an outage turns 257 recoverable jobs into 257 permanently failed
        // ones.
        let upstream = upstream_key(&resolved);
        if let Err(parked) = self.breakers.admit(&upstream) {
            // Tell this TENANT its work is paused, and tell only this tenant.
            //
            // The breaker is keyed by upstream and shared across every tenant
            // on the box; the tenant-visible `degraded` bit is not. Tying it to
            // the PARK rather than to "some breaker is open" is what keeps a
            // tenant with nothing in flight — or one that does not embed at all
            // — from seeing a banner claiming their processing is paused, which
            // for them would be false. The park is the exact moment the claim
            // becomes true.
            //
            // `upstream` is passed by value, not rebuilt: a key derived a
            // second time could drift from the one actually dialled, and the
            // node would then report healthy straight through an outage with no
            // error anywhere.
            crate::jobs::activity::JobActivityTracker::global().record_parked(context, &upstream);
            return Err(parked);
        }

        // Build the client from the resolution rather than from loose parts:
        // `base_url` and `dimensions` are both load-bearing for any endpoint
        // that is not the vendor's own (the built-in dimension tables only know
        // the vendors' model names), and passing the struct means a caller
        // cannot forget one.
        let provider = resolved.build()?;
        let model = resolved.model.clone();

        // Fetch node at exact revision
        let node = self
            .storage
            .nodes()
            .get(
                StorageScope::new(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    &context.workspace_id,
                ),
                node_id,
                Some(&context.revision),
            )
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node {} not found", node_id)))?;

        // The embedding TASKS this node carries — one per NAMED SPEC.
        //
        // A node is not limited to one vector any more. The default spec is the
        // node's own authored fields, exactly as before; a named spec is an
        // additional, separately-addressable embedding of some other text the
        // node carries — today the extraction artifact a binary asset holds in
        // `__extracted_text`, which is a Word document's body and has nothing
        // in common with the node's title and caption.
        //
        // They stay separate rather than being concatenated because they answer
        // different queries and want different chunking: a 40-page contract
        // glued onto a filename produces one vector that matches neither.
        let tasks = self.embedding_tasks(&node, &config, context).await?;

        if tasks.is_empty() {
            tracing::warn!(node_id = %node_id, "No embeddable content found, skipping");
            return Ok(());
        }

        // Embedder identity for multi-model support — derived from the
        // RESOLUTION, not from the config row.
        //
        // These two must describe the same model, because `provider` above is
        // what embeds the text and `embedder_id` is what the resulting vector
        // is filed under (`EmbedderId::to_key_hash` is the `{embedder_hash}`
        // segment of the `cf::EMBEDDINGS` key). Building one from `resolved`
        // and the other from `config` is how they came apart: in unified mode
        // (`ai_provider_ref` set) `config.provider` / `config.model` are the
        // untouched `TenantEmbeddingConfig::new` defaults — OpenAI /
        // text-embedding-3-small — so an Ollama tenant's vectors were stored
        // labelled as OpenAI's model, under OpenAI's hash, and repointing the
        // ref at another same-width model left them sharing one partition with
        // no error anywhere.
        //
        // The derivation itself lives in
        // `raisin_embeddings::resolve::ResolvedEmbeddingProvider::embedder_id`
        // so there stays exactly one of it; see its docs for why the provider
        // component must remain the enum variant name and never a slug.
        let embedder_id = resolved.embedder_id();

        // At info, because it is the one line that tells an operator which model
        // actually ran. `SHOW VECTOR INDEX HEALTH` reports a width and nothing
        // else, so before this a tenant could not distinguish "my new 1024-wide
        // model is live" from "the old 1024-wide one still is".
        tracing::info!(
            node_id = %node_id,
            specs = tasks.len(),
            embedder_provider = %embedder_id.provider,
            embedder_model = %embedder_id.model,
            embedder_dimensions = embedder_id.dimensions,
            embedder_hash = %embedder_id.to_key_hash(),
            "Embedder identity these vectors will be stored under"
        );

        // Chunking, routed the same way everything else about this node is.
        let chunking = self.resolve_chunking(&node, &config, context).await;

        for task in &tasks {
            self.embed_one_spec(
                node_id,
                task,
                &config,
                &embedder_id,
                chunking.as_ref(),
                provider.as_ref(),
                &upstream,
                context,
            )
            .await?;
        }

        Ok(())
    }

    /// Every named embedding this node should carry, with the text each one is
    /// made from.
    ///
    /// ONE derivation, so the writer, a future sweeper and any administrative
    /// regenerate cannot disagree about how many vectors a node has. A spec that
    /// yields no text is not returned at all — an empty spec must produce NO row
    /// rather than a row embedding the empty string, or every such node becomes
    /// a near-neighbour of every query.
    async fn embedding_tasks(
        &self,
        node: &raisin_models::nodes::Node,
        config: &TenantEmbeddingConfig,
        context: &JobContext,
    ) -> Result<Vec<EmbedTask>> {
        let mut tasks = Vec::new();

        // The default spec: the node's own authored fields, via the shape-driven
        // vector index plan. Unchanged, and still filed under the BARE node id,
        // which is what every existing row and every existing reader contains.
        let own = extract_embeddable_content(
            node,
            config,
            self.storage.clone(),
            context,
            &self.plan_cache,
        )
        .await?;
        if !own.trim().is_empty() {
            tasks.push(EmbedTask {
                spec: None,
                text: own,
            });
        }

        // The `doc` spec: the durable extraction artifact.
        //
        // Read through `raisin_models::nodes::extracted_text`, which returns
        // text ONLY for `__extract_status = 'ok'`. An `unsupported` node has an
        // empty `__extracted_text` and a durable record saying why; embedding
        // that would file a vector of nothing and erase the distinction.
        //
        // This is what makes PUBLISH cheap: the artifact travels in `properties`
        // with the SQL copy, so the publish branch builds this vector from text
        // that is already there — no extractor, no media plugin, no re-read of
        // the binary.
        if let Some(text) = raisin_models::nodes::extracted_text(&node.properties) {
            tasks.push(EmbedTask {
                spec: Some(EXTRACTED_TEXT_SPEC),
                text: text.to_string(),
            });
        }

        Ok(tasks)
    }

    /// Generate, store and index the vectors of ONE named spec for one node.
    ///
    /// Everything below is per-spec: the chunk plan, the spec hash, the
    /// staleness check, the HNSW ids and the orphan sweep. They are addressed
    /// through `raisin_hnsw::chunk_id`, whose `namespaced_source_id` puts the
    /// spec into the `cf::EMBEDDINGS` `{source_id}` key segment and leaves the
    /// default spec byte-identical to the bare node id it has always been — so
    /// N named embeddings per node is a key-VALUE change with no key-FORMAT
    /// change and no migration.
    #[allow(clippy::too_many_arguments)]
    async fn embed_one_spec(
        &self,
        node_id: &str,
        task: &EmbedTask,
        config: &TenantEmbeddingConfig,
        embedder_id: &raisin_ai::config::EmbedderId,
        chunking: Option<&raisin_ai::config::ChunkingConfig>,
        provider: &dyn raisin_embeddings::EmbeddingProviderTrait,
        upstream: &str,
        context: &JobContext,
    ) -> Result<()> {
        // A SECOND gate, and not a redundant one. `handle_generate` admits the
        // job once; a node with two specs then chunks twice, and if the first
        // spec's provider call was the one that tripped the breaker, the second
        // must not go on to chunk a 40-page document for a call that will not
        // be made. `check` is read-only — it never claims the half-open probe,
        // so a job cannot lock itself out halfway through its own attempt.
        self.breakers.check(upstream)?;

        let spec = task.spec;
        let text = &task.text;

        // The `{source_id}` key segment and the HNSW id space this spec owns.
        let source_id = raisin_hnsw::namespaced_source_id(node_id, spec);

        let text_preview: String = text.chars().take(200).collect();
        tracing::info!(
            node_id = %node_id,
            spec = %spec.unwrap_or("(default)"),
            text_length = text.len(),
            text_preview = %text_preview,
            "About to generate embedding for this text"
        );

        // Whether a byte range into `text` is worth recording.
        //
        // ONLY the `doc` spec. That spec embeds `__extracted_text` verbatim
        // (see `embedding_tasks`), so a range indexes a string that is stored on
        // the node and can be sliced again at query time. The DEFAULT spec's
        // text is synthesized by `extract_embeddable_content` and kept nowhere,
        // so a span recorded there would later be applied to
        // `__extracted_text` — a DIFFERENT string — and would return plausible
        // text from the wrong part of the document, with no error anywhere.
        let spans_are_durable = spec == Some(EXTRACTED_TEXT_SPEC);

        let span_of = |start: usize, end: usize| -> Option<raisin_embeddings::ChunkSpan> {
            if spans_are_durable {
                raisin_embeddings::ChunkSpan::new(start, end)
            } else {
                None
            }
        };

        // The whole text as one chunk: the range is trivially the whole string.
        let whole = |text: &String| vec![(text.clone(), 0usize, span_of(0, text.len()))];

        // Split text into chunks if chunking is configured
        type Chunk = (String, usize, Option<raisin_embeddings::ChunkSpan>);
        let chunks: Vec<Chunk> = if let Some(chunking_config) = chunking {
            match raisin_ai::chunking::TextChunker::chunk_text(text, chunking_config) {
                Ok(text_chunks) if text_chunks.len() > 1 => {
                    tracing::info!(
                        node_id = %node_id,
                        spec = %spec.unwrap_or("(default)"),
                        chunk_count = text_chunks.len(),
                        "Split text into {} chunks for embedding",
                        text_chunks.len()
                    );
                    text_chunks
                        .into_iter()
                        .map(|c| {
                            let span = span_of(c.start_offset, c.end_offset);
                            (c.content, c.index, span)
                        })
                        .collect()
                }
                Ok(_) => whole(text),
                Err(e) => {
                    tracing::warn!(
                        node_id = %node_id,
                        error = %e,
                        "Chunking failed, falling back to single embedding"
                    );
                    whole(text)
                }
            }
        } else {
            whole(text)
        };

        let total_chunks = chunks.len();

        // THE staleness identity: every input that decided these vectors, in one
        // hash. Computed from the same `text`, `embedder_id` and `chunking` the
        // lines above resolved — not re-derived — so what is stored describes
        // what actually ran. The spec NAME is part of it too, so a row can be
        // checked against the spec it claims to be and not only against where it
        // happens to sit.
        let spec_hash = raisin_embeddings::EmbeddingSpec::new(text, embedder_id, chunking)
            .for_spec(spec)
            .hash();
        let embedder_hash = embedder_id.to_key_hash();
        let kind_char = raisin_ai::config::EmbeddingKind::Text.to_key_char();

        // The SAME two segments, rendered once, as the HNSW index's partition.
        // `EmbeddingPartition::to_index_token` is the only rendering there is,
        // and a test in `repositories::embedding_storage::storage` asserts its
        // bytes equal segments 5 and 6 of the key `store_embedding` writes — so
        // a vector cannot be stored under one identity and indexed under
        // another.
        let partition = raisin_hnsw::PartitionId::new(
            raisin_ai::config::EmbeddingPartition::new(
                embedder_id.clone(),
                raisin_ai::config::EmbeddingKind::Text,
            )
            .to_index_token(),
        );

        // The HNSW ids this run owns. Note the two vocabularies: a chunk is a
        // `{node}[#{spec}]#{n}` ID in the index, but in `cf::EMBEDDINGS` it is
        // the spec's source id plus a chunk-index key segment. Both have to be
        // swept, and each by its own rule.
        let live_ids: std::collections::HashSet<String> =
            raisin_hnsw::chunk_id_set(node_id, spec, total_chunks)
                .into_iter()
                .collect();

        let force = force_requested(context);

        // Steady state: identical inputs, already done, nothing to do.
        //
        // This is what stops a periodic re-embed pass from calling the provider
        // for a corpus that has not changed and rewriting every row it reads —
        // and it is only sound because the comparison is against the SPEC, not
        // the text. A text-only check would call every re-chunked document
        // unchanged for the chunks whose boundaries happened not to move.
        if !force
            && self
                .is_already_current(
                    &source_id,
                    context,
                    &embedder_hash,
                    kind_char,
                    &partition,
                    &live_ids,
                    spec_hash,
                    total_chunks,
                )
                .await?
        {
            tracing::info!(
                node_id = %node_id,
                spec = %spec.unwrap_or("(default)"),
                total_chunks = total_chunks,
                spec_hash = spec_hash,
                "Embedding is current for these inputs — skipping (no provider call, no write)"
            );
            return Ok(());
        }

        let embedding_storage =
            crate::repositories::RocksDBEmbeddingStorage::new(self.storage.db().clone());

        // What is indexed under this source but is no longer live. COMPUTED
        // here, APPLIED below — the unchanged-content fast path needs to know
        // whether anything must be evicted before it can decide to skip, and
        // asking that question must not itself perform the eviction.
        let stale =
            self.stale_chunk_ids(&source_id, context, &embedder_hash, kind_char, &live_ids)?;

        // CARRY FORWARD: the same inputs at an EARLIER revision.
        //
        // `is_already_current` is pinned to `context.revision`, so any rewrite of
        // the node — a publish that copies identical properties onto another
        // branch, a save that touched a field nothing embeds — misses it and
        // would call the provider again for text that has not changed. The rows
        // are keyed by revision so a new row is genuinely needed; the VECTOR is
        // not, and a vector is the expensive part.
        //
        // This is what makes "publish carries the artifact" pay off twice: the
        // first publish embeds once, and every re-publish of unchanged content
        // costs zero provider calls. Note the limit — rows are branch-scoped, so
        // this reuses within a branch (and across a FORK, which copies
        // `cf::EMBEDDINGS`), never from another branch's rows.
        let carried = if force {
            None
        } else {
            Self::carry_forward_vectors(
                &embedding_storage,
                context,
                &embedder_hash,
                kind_char,
                &source_id,
                spec_hash,
                total_chunks,
            )
        };

        // STEADY STATE: a revision bump whose embeddable content did not change.
        //
        // `is_already_current` is pinned to `context.revision`, so a rewrite of
        // the node always misses it and lands here instead. `carry_forward`
        // already spares the provider call — but the per-chunk index loop below
        // still ran, doing one remove + one add of the same id per revision.
        // That was pure churn, and on usearch < 2.26 it was the churn that
        // saturated the key table and wedged the index: a vmount syncing every
        // ~15-20s got there in about 85 minutes.
        //
        // The row write below is NOT skipped — rows are revision-keyed and the
        // new row is genuinely needed, and a later REBUILD reads them. Only the
        // index mutation is skipped, and only when all four hold:
        //
        //   1. not forced — an operator asking for a re-embed always gets one;
        //   2. the vectors carried forward, i.e. every chunk's newest row is
        //      current for this spec hash and chunk count, so what we would
        //      write is byte-identical to what is already there;
        //   3. nothing is stale — a non-empty set means the chunking changed
        //      and ids must be evicted, which the skip would never do;
        //   4. every live id is actually resident in the index right now, which
        //      is what covers the snapshot lag (see `all_chunks_resident`).
        //
        // Anything less than all four takes the full path. Failing open here
        // costs one redundant index write; failing closed would cost a silently
        // stale index, which nobody notices until a search comes back short.
        //
        // Residual, and it is the same one the `is_already_current` fast path
        // already carries: a crash BETWEEN the row write and the index write
        // leaves a row whose vector is resident but stale. Condition 4 catches a
        // MISSING vector, not a stale one. This does not widen that window, it
        // just makes it reachable more often.
        let skip_index_write = !force
            && carried.is_some()
            && stale.is_empty()
            && self
                .all_chunks_resident(context, &partition, &live_ids)
                .await?;

        if skip_index_write {
            tracing::debug!(
                node_id = %node_id,
                spec = %spec.unwrap_or("(default)"),
                total_chunks = total_chunks,
                "Content unchanged and every chunk already resident — writing the revision's \
                 rows but leaving the HNSW index alone"
            );
        } else {
            self.remove_stale_chunks_from_hnsw(&source_id, context, &partition, &stale)
                .await?;
        }

        let carried_forward = carried.is_some();
        let chunk_texts: Vec<String> = chunks
            .iter()
            .map(|(content, _, _)| content.clone())
            .collect();
        let embeddings = match carried {
            Some(vectors) => {
                tracing::info!(
                    node_id = %node_id,
                    spec = %spec.unwrap_or("(default)"),
                    total_chunks = total_chunks,
                    spec_hash = spec_hash,
                    "Reused the vectors of an earlier revision — same inputs, no provider call"
                );
                vectors
            }
            None => {
                // `generate_embeddings_for_all`, not `..._batch`: a chunked
                // document routinely exceeds the endpoint's per-request input
                // limit, and the raw batch call sends the whole set in one go.
                // That is a 400, not a slow request — the job fails, retries and
                // fails forever, so the document's vectors are never written
                // while the default spec's single filename vector is.
                //
                // The breaker is told the OUTCOME here, and nowhere else. This
                // is the only line in the job that actually talks to the
                // upstream, so it is the only line that has evidence about the
                // upstream's health — recording anywhere else would be
                // recording a guess. Note that `record_failure` classifies via
                // `Error::is_upstream_fault`, so a 4xx passes through it
                // without opening anything.
                let outcome = if chunk_texts.len() > 1 {
                    provider.generate_embeddings_for_all(&chunk_texts).await
                } else {
                    provider
                        .generate_embedding(&chunk_texts[0])
                        .await
                        .map(|v| vec![v])
                };
                let mut generated = match outcome {
                    Ok(vectors) => {
                        self.breakers.record_success(upstream);
                        // The tenant's work is moving again. Clearing on the
                        // first success rather than waiting for the publisher's
                        // periodic re-check is what makes the banner disappear
                        // when the outage ends, not up to a tick later.
                        crate::jobs::activity::JobActivityTracker::global()
                            .record_upstream_ok(context, upstream);
                        vectors
                    }
                    Err(e) => {
                        self.breakers.record_failure(upstream, &e);
                        return Err(e);
                    }
                };
                for emb in &mut generated {
                    *emb = raisin_hnsw::normalize_vector(emb);
                }
                tracing::info!(
                    node_id = %node_id,
                    spec = %spec.unwrap_or("(default)"),
                    embedding_dims = generated[0].len(),
                    total_chunks = total_chunks,
                    text_length = text.len(),
                    "Successfully generated and normalized {} embedding(s)",
                    generated.len()
                );
                generated
            }
        };

        // Store each chunk and add to HNSW
        for ((chunk_content, chunk_index, chunk_span), embedding) in
            chunks.iter().zip(embeddings.into_iter())
        {
            // ONE derivation of the chunk id, shared with the orphan sweep and
            // the HNSW cleanup. A `format!` here and a second one there is
            // exactly how a chunk becomes unreachable by the code meant to
            // delete it.
            let chunk_id = raisin_hnsw::chunk_source_id(node_id, spec, *chunk_index, total_chunks);

            #[allow(deprecated)]
            let embedding_data = EmbeddingData {
                vector: embedding.clone(),
                embedder_id: embedder_id.clone(),
                embedding_kind: raisin_ai::config::EmbeddingKind::Text,
                // The NAMESPACED source id: `{node}` for the default spec,
                // `{node}#{spec}` for a named one. This is the `{source_id}`
                // key segment, and it is the whole of the namespacing change.
                source_id: source_id.clone(),
                chunk_index: *chunk_index,
                total_chunks,
                chunk_content: Some(chunk_content.chars().take(200).collect()),
                generated_at: chrono::Utc::now(),
                // Per-chunk text hash: unchanged meaning, a debugging aid.
                text_hash: hash_text(chunk_content),
                // Per-SPEC spec hash: the answer to "is this stale?". Every
                // chunk of one run carries the same value, because they are all
                // the product of the same inputs — that is what lets a single
                // read of the first chunk decide the whole spec.
                spec_hash: Some(spec_hash),
                // The byte range this chunk occupies in the text it was cut
                // from — `None` for the default spec, whose text is not
                // stored and therefore cannot be sliced again.
                chunk_span: *chunk_span,
                // Legacy fields (deprecated)
                model: config.model.clone(),
                provider: config.provider.clone(),
            };

            embedding_storage.store_embedding(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &context.workspace_id,
                &chunk_id,
                &context.revision,
                &embedding_data,
            )?;

            // Add to HNSW index (use spawn_blocking as HNSW operations are sync)
            if !skip_index_write {
                let engine_clone = Arc::clone(&self.hnsw_engine);
                let index_id = chunk_id.clone();
                let tenant_id = context.tenant_id.clone();
                let repo_id = context.repo_id.clone();
                let branch = context.branch.clone();
                let workspace_id = context.workspace_id.clone();
                let revision = context.revision;
                let partition_clone = partition.clone();

                tokio::task::spawn_blocking(move || {
                    engine_clone.add_embedding(
                        &tenant_id,
                        &repo_id,
                        &branch,
                        &partition_clone,
                        &workspace_id,
                        &index_id,
                        revision,
                        embedding,
                    )
                })
                .await
                .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;
            }
        }

        // ONE line per job when the index was actually mutated, at info.
        //
        // The only indexing signal here used to be `debug!`, which is why 25
        // minutes of a wedged production index produced a completely silent log.
        // Under the skip above a real mutation is rare — content changes only —
        // so this stays quiet in the steady state rather than one line per node
        // per sync tick.
        if !skip_index_write {
            tracing::info!(
                node_id = %node_id,
                spec = %spec.unwrap_or("(default)"),
                total_chunks = total_chunks,
                evicted = stale.len(),
                source = if carried_forward { "carried" } else { "generated" },
                "Wrote {} chunk(s) to the HNSW index",
                total_chunks
            );
        }

        // Orphan sweep, AFTER the new rows are in: re-chunking into fewer pieces
        // leaves the surplus high-index rows of THIS revision behind, and they
        // keep matching queries forever — anything that rebuilds the index reads
        // them straight back out of RocksDB, so removing them from the index
        // alone is not a fix. Rows at OTHER revisions are that revision's
        // correct content and are left exactly where they are.
        let pruned = embedding_storage.prune_chunks_from(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            &context.workspace_id,
            &embedder_hash,
            kind_char,
            &source_id,
            total_chunks,
            &context.revision,
        )?;
        if !pruned.is_empty() {
            tracing::info!(
                node_id = %node_id,
                spec = %spec.unwrap_or("(default)"),
                total_chunks = total_chunks,
                orphans = ?pruned,
                "Deleted orphaned chunk rows left by the previous chunking at this revision"
            );
        }

        Ok(())
    }

    /// The vectors of an EARLIER revision, when the inputs have not moved.
    ///
    /// Returns `Some(vectors)` only when every chunk of the newest stored
    /// revision of this spec carries this exact spec hash and chunk count.
    /// Anything less returns `None`, so the outcome of a doubt is a provider
    /// call and never a wrong vector.
    ///
    /// # Why the exact-revision check is not enough on its own
    ///
    /// `is_already_current` reads the row AT `context.revision`. Every rewrite
    /// of a node mints a new revision, so it misses on: a Studio publish (a SQL
    /// copy of identical properties onto the `publish` branch), a save that
    /// touched a field nothing embeds, a replicated write. In each of those the
    /// text, the embedder and the chunking are byte-identical and the vector
    /// would come back identical too — for the price of a provider call per
    /// chunk, per node, per publish.
    ///
    /// The ROW still has to be written (revision is part of the key, and the
    /// vector legs read the latest row per source id). It is the VECTOR that is
    /// carried, which is the part that costs money and latency.
    fn carry_forward_vectors(
        embedding_storage: &crate::repositories::RocksDBEmbeddingStorage,
        context: &JobContext,
        embedder_hash: &str,
        kind_char: char,
        source_id: &str,
        spec_hash: u64,
        total_chunks: usize,
    ) -> Option<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(total_chunks);
        for chunk_index in 0..total_chunks {
            let stored = embedding_storage
                .get_chunk(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    &context.workspace_id,
                    embedder_hash,
                    kind_char,
                    source_id,
                    chunk_index,
                    // Latest revision, whatever it is — the whole point is that
                    // it is NOT this one.
                    None,
                )
                .ok()
                .flatten()?;
            if !stored.is_current_for(spec_hash, total_chunks) || stored.vector.is_empty() {
                return None;
            }
            vectors.push(stored.vector);
        }
        Some(vectors)
    }

    /// Does a stored embedding already match these inputs exactly?
    ///
    /// Three things have to agree, and dropping any one of them is a bug in a
    /// different direction:
    ///
    /// - every chunk row of this revision must exist and carry this spec hash
    ///   and chunk count, or the inputs have moved and the vectors are stale;
    /// - there must be NO row above the new chunk count at this revision, or a
    ///   previous, longer chunking has left orphans that still need sweeping;
    /// - every live chunk id must actually be IN the HNSW index. Snapshots lag
    ///   (`raisin-hnsw` `lifecycle.rs`), so a restart can leave the row present
    ///   and the vector gone. Skipping on the rows alone would make that node
    ///   permanently unsearchable until a full REBUILD.
    #[allow(clippy::too_many_arguments)]
    async fn is_already_current(
        &self,
        source_id: &str,
        context: &JobContext,
        embedder_hash: &str,
        kind_char: char,
        partition: &raisin_hnsw::PartitionId,
        live_ids: &std::collections::HashSet<String>,
        spec_hash: u64,
        total_chunks: usize,
    ) -> Result<bool> {
        let embedding_storage =
            crate::repositories::RocksDBEmbeddingStorage::new(self.storage.db().clone());

        for chunk_index in 0..total_chunks {
            let stored = embedding_storage.get_chunk(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &context.workspace_id,
                embedder_hash,
                kind_char,
                source_id,
                chunk_index,
                Some(&context.revision),
            )?;
            match stored {
                Some(data) if data.is_current_for(spec_hash, total_chunks) => {}
                _ => return Ok(false),
            }
        }

        // An orphan of a previous, longer chunking is work still outstanding at
        // this revision, so it is not a steady state however current the rows
        // below it look.
        let has_orphan = embedding_storage
            .list_source_chunks(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &context.workspace_id,
                embedder_hash,
                kind_char,
                source_id,
            )?
            .into_iter()
            .any(|(chunk_index, revision)| {
                chunk_index >= total_chunks && revision == context.revision
            });
        if has_orphan {
            return Ok(false);
        }

        self.all_chunks_resident(context, partition, live_ids).await
    }

    /// Is every live chunk id actually IN the index right now?
    ///
    /// The rows in `cf::EMBEDDINGS` and the vectors in the HNSW index are two
    /// separate stores, and the index snapshots lag (~60s). A restart can
    /// therefore leave the row present and the vector gone, so any fast path
    /// that decides "nothing to do" from the rows alone would make that node
    /// permanently unsearchable until a full `REBUILD VECTOR INDEX`. This is the
    /// check that stops it, and it is deliberately ONE implementation shared by
    /// both fast paths — a second copy is how the two would drift into
    /// disagreeing about what "already indexed" means.
    async fn all_chunks_resident(
        &self,
        context: &JobContext,
        partition: &raisin_hnsw::PartitionId,
        live_ids: &std::collections::HashSet<String>,
    ) -> Result<bool> {
        let engine = Arc::clone(&self.hnsw_engine);
        let tenant_id = context.tenant_id.clone();
        let repo_id = context.repo_id.clone();
        let branch = context.branch.clone();
        let ids: Vec<String> = live_ids.iter().cloned().collect();
        let partition = partition.clone();

        tokio::task::spawn_blocking(move || -> Result<bool> {
            for id in &ids {
                if !engine.contains_embedding(&tenant_id, &repo_id, &branch, &partition, id)? {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .await
        .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))?
    }

    /// Handle embedding deletion job
    ///
    /// This method:
    /// 1. Extracts node_id from JobType::EmbeddingDelete
    /// 2. Checks for existing chunk count via embedding storage
    /// 3. Removes all chunks (or single embedding) from the HNSW index
    /// 4. Removes the same rows from `cf::EMBEDDINGS`
    ///
    /// Step 4 is not optional. Dropping only the HNSW entry leaves the stored
    /// vector behind, and `cf::EMBEDDINGS` is the source of truth every
    /// administrative operation reads: `VERIFY VECTOR INDEX` then reports a
    /// permanent `mismatch` (hnsw_count < storage_count) telling the operator to
    /// run `REBUILD VECTOR INDEX`, and the rebuild dutifully RE-INSERTS the
    /// deleted node's vector. Its content does not leak — the row-fetch stage
    /// drops it with a `Node not found` warning — but it wins ANN slots that
    /// belong to live documents, so a `LIMIT k` silently returns fewer than k
    /// rows. Measured on the unpublish path (`DELETE ... WHERE __branch=`), where
    /// the residue accumulates once per unpublished node and recall degrades
    /// with every one.
    pub async fn handle_delete(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract node_id from job type
        let node_id = match &job.job_type {
            JobType::EmbeddingDelete { node_id } => node_id,
            _ => {
                return Err(Error::Validation(format!(
                    "Expected EmbeddingDelete job, got {}",
                    job.job_type
                )))
            }
        };

        tracing::debug!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            workspace_id = %context.workspace_id,
            node_id = %node_id,
            "Processing embedding deletion job"
        );

        // TWO VOCABULARIES, and they are not interchangeable — which is the
        // trap this function used to fall into by iterating one list against
        // both stores.
        //
        // * the HNSW index is keyed by INDEX ID: `{node}`, `{node}#{n}`, or
        //   `{node}#{spec}#{n}`;
        // * `cf::EMBEDDINGS` is keyed by SOURCE ID (`{node}` or
        //   `{node}#{spec}`) with the chunk index as a SEPARATE key segment, so
        //   one delete per source id removes every chunk at every revision.
        //
        // Handing an index id to the row store matches nothing. The default
        // spec got away with it only because the bare node id happens to be
        // both, and it is exactly what left a named spec's rows behind: the
        // vectors survived, `VERIFY VECTOR INDEX` reported a permanent
        // mismatch, and a REBUILD re-inserted the deleted node's document
        // vector so it went on winning ANN slots that belong to live content.
        //
        // Both lists cover EVERY spec the node may carry. A deleted node must
        // lose all of its vectors, not the ones the default spec happened to
        // write.
        let mut index_ids: Vec<String> = Vec::new();
        let mut source_ids: Vec<String> = Vec::new();
        for spec in [None, Some(EXTRACTED_TEXT_SPEC)] {
            let source_id = raisin_hnsw::namespaced_source_id(node_id, spec);
            let total_chunks = self.get_existing_chunk_count(&source_id, context);

            // The bare node id is an index id unconditionally: it is both the
            // single-chunk DEFAULT id and the legacy non-chunked id, and a node
            // that was once written unchunked and later chunked has entries
            // under both.
            if spec.is_none() {
                index_ids.push(node_id.clone());
            }
            index_ids.extend(raisin_hnsw::chunk_id_set(node_id, spec, total_chunks));
            source_ids.push(source_id);
        }
        index_ids.sort();
        index_ids.dedup();
        source_ids.sort();
        source_ids.dedup();

        let engine_clone = Arc::clone(&self.hnsw_engine);
        let db = self.storage.db().clone();
        let tenant_id = context.tenant_id.clone();
        let repo_id = context.repo_id.clone();
        let branch = context.branch.clone();
        let workspace_id = context.workspace_id.clone();
        let index_ids_clone = index_ids.clone();
        let source_ids_clone = source_ids.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let embedding_storage = crate::repositories::RocksDBEmbeddingStorage::new(db);
            // EVERY partition, not the one this tenant currently writes. A
            // deleted node must lose all of its vectors — including a previous
            // model's, and (once image embedding exists) its image vector.
            // Removing an id a partition does not hold is a no-op, so the sweep
            // is safe as well as complete; a partition-blind delete would leave
            // a deleted node winning ANN slots forever, which is the same class
            // of residue the spec-blind delete used to leave.
            let partitions = engine_clone.list_partitions(&tenant_id, &repo_id, &branch)?;
            for partition in &partitions {
                for id in &index_ids_clone {
                    engine_clone.remove_embedding(&tenant_id, &repo_id, &branch, partition, id)?;
                }
            }
            for source_id in &source_ids_clone {
                // `revision: None` purges EVERY stored revision of this source
                // id on this branch — which is what a delete means here. Older
                // revisions are not reachable state for search: the vector legs
                // (live query, VERIFY, REBUILD) all read the latest row per
                // source id, so leaving them would recreate the exact residue
                // this call exists to remove.
                embedding_storage.delete_embedding(
                    &tenant_id,
                    &repo_id,
                    &branch,
                    &workspace_id,
                    source_id,
                    None,
                )?;
            }

            Ok(())
        })
        .await
        .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::debug!(
            node_id = %node_id,
            index_ids = index_ids.len(),
            source_ids = ?source_ids,
            "Removed embedding(s) from the HNSW index and from cf::EMBEDDINGS"
        );

        Ok(())
    }

    /// Look up the existing chunk count for one SPEC of a node.
    ///
    /// `source_id` is the namespaced id (`{node}` or `{node}#{spec}`), because
    /// the chunk count is per-spec: a page's own two chunks say nothing about
    /// how its attached document was split.
    fn get_existing_chunk_count(&self, source_id: &str, context: &JobContext) -> usize {
        let embedding_storage =
            crate::repositories::RocksDBEmbeddingStorage::new(self.storage.db().clone());

        for candidate in [format!("{}#0", source_id), source_id.to_string()] {
            if let Ok(Some(data)) = embedding_storage.get_embedding(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &context.workspace_id,
                &candidate,
                None,
            ) {
                return data.total_chunks;
            }
        }

        1
    }

    /// Drop from the HNSW index every id of this node that the run about to
    /// happen will NOT rewrite.
    ///
    /// The candidate set comes from what is actually STORED — the chunk indexes
    /// this node has ever been written under, at any revision — rather than from
    /// arithmetic over a recorded chunk count. Two cases the old count-based
    /// version got wrong, both leaving a stale vector answering queries:
    ///
    /// - a document that was ONE embedding and is now chunked: the old code
    ///   returned early on `old_total <= 1` saying "add() replaces it", but the
    ///   new vectors go in under `{node}#0…`, so the whole-document vector under
    ///   the bare id was never replaced and never removed;
    /// - a document re-chunked into FEWER pieces where the surplus indexes were
    ///   written by an earlier revision: `total_chunks` on the latest row no
    ///   longer names them, so `0..old_total` never reached them.
    ///
    /// Removing an id the index does not hold is a no-op, so covering both id
    /// forms costs nothing and closes the case where a row's own chunk count
    /// cannot be known from its key.
    #[allow(clippy::too_many_arguments)]
    async fn remove_old_chunks_from_hnsw(
        &self,
        source_id: &str,
        context: &JobContext,
        embedder_hash: &str,
        kind_char: char,
        partition: &raisin_hnsw::PartitionId,
        live_ids: &std::collections::HashSet<String>,
    ) -> Result<()> {
        let stale = self.stale_chunk_ids(source_id, context, embedder_hash, kind_char, live_ids)?;
        self.remove_stale_chunks_from_hnsw(source_id, context, partition, &stale)
            .await
    }

    /// Which chunk ids are indexed under this source but are no longer live.
    ///
    /// Split out from the removal so a caller can ask "is there anything to
    /// evict?" WITHOUT evicting. That question is one of the preconditions for
    /// skipping the index write entirely on an unchanged re-embed: a non-empty
    /// answer means the chunking changed, and the full path must run.
    fn stale_chunk_ids(
        &self,
        source_id: &str,
        context: &JobContext,
        embedder_hash: &str,
        kind_char: char,
        live_ids: &std::collections::HashSet<String>,
    ) -> Result<Vec<String>> {
        let embedding_storage =
            crate::repositories::RocksDBEmbeddingStorage::new(self.storage.db().clone());

        let stored = embedding_storage.list_source_chunks(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            &context.workspace_id,
            embedder_hash,
            kind_char,
            source_id,
        )?;

        // The bare id (how a single-chunk DEFAULT-spec run indexes) plus every
        // chunk index ever stored for this spec (how every other run does).
        //
        // Built from the SOURCE id, so a named spec sweeps its own namespace and
        // cannot reach into the default one — `{node}#doc#0` and `{node}#0` are
        // different vectors of the same node and neither supersedes the other.
        let parsed = raisin_hnsw::parse_index_id(source_id);
        let spec = parsed.spec.as_deref();
        // The bare id is a candidate only for the DEFAULT spec, where it is a
        // real index id (a single-chunk run, and every legacy row). A NAMED
        // spec always writes `{node}#{spec}#{n}`, so `{node}#{spec}` is not an
        // id anything ever indexed and listing it would sweep a string that
        // cannot exist.
        let mut candidates: Vec<String> = if spec.is_none() {
            vec![source_id.to_string()]
        } else {
            Vec::new()
        };
        for (chunk_index, _) in &stored {
            candidates.push(raisin_hnsw::chunk_index_id(
                &parsed.node_id,
                spec,
                *chunk_index,
            ));
        }
        candidates.sort();
        candidates.dedup();
        Ok(candidates
            .into_iter()
            .filter(|id| !live_ids.contains(id))
            .collect())
    }

    /// Evict the ids `stale_chunk_ids` identified.
    async fn remove_stale_chunks_from_hnsw(
        &self,
        source_id: &str,
        context: &JobContext,
        partition: &raisin_hnsw::PartitionId,
        stale: &[String],
    ) -> Result<()> {
        if stale.is_empty() {
            return Ok(());
        }

        let engine_clone = Arc::clone(&self.hnsw_engine);
        let tenant_id = context.tenant_id.clone();
        let repo_id = context.repo_id.clone();
        let branch = context.branch.clone();
        let to_remove = stale.to_vec();
        let partition = partition.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            for id in &to_remove {
                engine_clone.remove_embedding(&tenant_id, &repo_id, &branch, &partition, id)?;
            }
            Ok(())
        })
        .await
        .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::debug!(
            source_id = %source_id,
            removed = ?stale,
            "Removed superseded chunk ids from the HNSW index before re-embedding"
        );

        Ok(())
    }

    /// Handle branch copy job
    ///
    /// This method:
    /// 1. Extracts source_branch from JobType::EmbeddingBranchCopy
    /// 2. Copies HNSW index for the new branch
    pub async fn handle_branch_copy(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Extract source_branch from job type
        let source_branch = match &job.job_type {
            JobType::EmbeddingBranchCopy { source_branch } => source_branch,
            _ => {
                return Err(Error::Validation(format!(
                    "Expected EmbeddingBranchCopy job, got {}",
                    job.job_type
                )))
            }
        };

        tracing::debug!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            source_branch = %source_branch,
            new_branch = %context.branch,
            workspace_id = %context.workspace_id,
            "Processing embedding branch copy job"
        );

        let engine_clone = Arc::clone(&self.hnsw_engine);
        let tenant_id = context.tenant_id.clone();
        let repo_id = context.repo_id.clone();
        let new_branch = context.branch.clone();
        let source_branch_clone = source_branch.clone();

        tokio::task::spawn_blocking(move || {
            engine_clone.copy_for_branch(&tenant_id, &repo_id, &source_branch_clone, &new_branch)
        })
        .await
        .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::info!(
            branch = %context.branch,
            source_branch = %source_branch,
            "Copied HNSW index for new branch"
        );

        Ok(())
    }

    /// Resolve the chunking configuration for one node.
    ///
    /// # Why chunking is routed and not merely global
    ///
    /// One chunk size cannot be right for everything a repo holds. A scanned
    /// contract wants large, overlapping chunks so a clause survives a page
    /// break; a product description wants one chunk and no overlap, because
    /// splitting it makes every fragment match everything. Since the routing
    /// table already answers "what kind of content is this" per mimetype and
    /// per path, it is also the right place to answer "and how should it be
    /// split" — which is what a user customising chunking is actually asking
    /// for.
    ///
    /// # This field was already there, and already dead
    ///
    /// `ProcessingSettings::chunking` has been persisted in
    /// `CF_PROCESSING_RULES`, editable over the processing-rules HTTP surface,
    /// and covered by a round-trip test in `repositories/processing_rules.rs` —
    /// and read by nothing. An operator could set it, save it, reload it and
    /// see it come back, while every embedding was chunked by the tenant-wide
    /// setting. Threading it here is what makes that surface mean something; it
    /// is not a new concept.
    ///
    /// Precedence is rule-then-tenant, most specific first, and the tenant value
    /// remains the answer when no rule sets one — so an installation that never
    /// opens the rules editor is unaffected.
    ///
    /// This deliberately applies to ANY node, not only a `raisin:Asset`. A rule
    /// can match on node type, path or workspace, so "everything under
    /// `/handbook/**` chunks at 1024" is expressible for ordinary content nodes
    /// that have no binary at all. `mime_type` is simply absent for those, and a
    /// mimetype matcher then does not match.
    async fn resolve_chunking(
        &self,
        node: &raisin_models::nodes::Node,
        config: &TenantEmbeddingConfig,
        context: &JobContext,
    ) -> Option<raisin_ai::config::ChunkingConfig> {
        // `get_rules` lives on the trait, not the impl.
        use raisin_storage::ProcessingRulesRepository as _;

        let rule_chunking = match self
            .storage
            .processing_rules_repository()
            .get_rules(raisin_storage::RepoScope::new(
                &context.tenant_id,
                &context.repo_id,
            ))
            .await
        {
            Ok(Some(rule_set)) => {
                let mut match_context = raisin_ai::RuleMatchContext::new()
                    .with_node_type(&node.node_type)
                    .with_path(&node.path)
                    .with_workspace(&context.workspace_id);
                // The same accessor the enqueue gate and the asset job use, so a
                // mimetype rule selects the same rule for chunking that it
                // selects for extraction. A second, narrower reader here is
                // exactly how the two would come apart.
                if let Some(mime) =
                    crate::jobs::handlers::asset_processing::helpers::extract_mime_type(node)
                {
                    match_context = match_context.with_mime_type(mime);
                }
                rule_set
                    .find_matching_rule(&match_context)
                    .and_then(|r| r.settings.chunking.clone())
            }
            _ => None,
        };

        if let Some(chunking) = rule_chunking {
            tracing::info!(
                node_id = %node.id,
                node_path = %node.path,
                chunk_size = chunking.chunk_size,
                "Chunking taken from the matching processing rule"
            );
            return Some(chunking);
        }

        config.chunking.clone()
    }

    /// Resolve the embedding provider configuration.
    ///
    /// Delegates to the ONE resolver — `raisin_embeddings::resolve` via
    /// [`crate::embedding_provider`] — which every read-path surface also goes
    /// through. This method used to be the only correct derivation of five;
    /// keeping it as a thin wrapper is what stops it becoming that again.
    async fn resolve_embedding_provider(
        &self,
        config: &TenantEmbeddingConfig,
        tenant_id: &str,
    ) -> Result<ResolvedEmbeddingProvider> {
        crate::embedding_provider::resolve_settings(
            &self.storage,
            tenant_id,
            config,
            &self.master_key,
        )
        .await
    }
}

/// Did the enqueuer ask for an unconditional re-embed?
///
/// The key is [`raisin_embeddings::FORCE_REEMBED_KEY`] — defined in the crate
/// the enqueuing HTTP handler also depends on, so the two cannot spell it
/// differently.
fn force_requested(context: &JobContext) -> bool {
    context
        .metadata
        .get(raisin_embeddings::FORCE_REEMBED_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}
