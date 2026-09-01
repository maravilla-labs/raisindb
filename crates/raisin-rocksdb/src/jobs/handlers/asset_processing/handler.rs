//! AssetProcessingHandler struct, model management, and job handling

use raisin_ai::{ClipEmbedder, ModelRegistry};
use raisin_core::services::node_service::NodeService;
use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::helpers::{extract_mime_type, extract_storage_key, is_image_mime, process_extractable};
use super::types::{AssetProcessingResult, BinaryRetrievalCallback};
use super::{ExtractStatus, ExtractionArtifact};
use crate::RocksDBStorage;

/// Handler for automatic asset processing jobs.
///
/// # Deprecation Notice
///
/// This handler is deprecated in favor of user-defined trigger functions that use
/// the Resource API and `raisin.ai.*` SDK methods.
///
/// See `examples/launchpad/package/content/functions/lib/launchpad/process-asset/`
#[deprecated(
    since = "0.12.0",
    note = "Use Resource API and raisin.ai.* with user-defined triggers instead. See process-asset example."
)]
pub struct AssetProcessingHandler {
    storage: Arc<RocksDBStorage>,
    binary_callback: Option<BinaryRetrievalCallback>,
    model_registry: Arc<RwLock<Option<ModelRegistry>>>,
    clip_embedder: Arc<RwLock<Option<ClipEmbedder>>>,
}

impl AssetProcessingHandler {
    /// Create a new asset processing handler
    pub fn new(storage: Arc<RocksDBStorage>) -> Self {
        Self {
            storage,
            binary_callback: None,
            model_registry: Arc::new(RwLock::new(None)),
            clip_embedder: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the binary retrieval callback
    pub fn with_binary_callback(mut self, callback: BinaryRetrievalCallback) -> Self {
        self.binary_callback = Some(callback);
        self
    }

    /// Set the binary retrieval callback (mutable)
    pub fn set_binary_callback(&mut self, callback: BinaryRetrievalCallback) {
        self.binary_callback = Some(callback);
    }

    /// Initialize the model registry
    async fn ensure_model_registry(&self) -> Result<()> {
        let mut registry = self.model_registry.write().await;
        if registry.is_none() {
            let new_registry = ModelRegistry::new()
                .map_err(|e| Error::Backend(format!("Failed to create model registry: {}", e)))?;
            new_registry.refresh_download_status().await;
            *registry = Some(new_registry);
        }
        Ok(())
    }

    /// Get or download and load the CLIP embedder
    async fn get_or_load_clip(&self) -> Result<()> {
        {
            let embedder = self.clip_embedder.read().await;
            if embedder.is_some() {
                return Ok(());
            }
        }

        self.ensure_model_registry().await?;

        let model_id = "openai/clip-vit-base-patch32";
        let model_path: PathBuf;

        {
            let registry_guard = self.model_registry.read().await;
            let registry = registry_guard
                .as_ref()
                .ok_or_else(|| Error::Backend("Model registry not initialized".to_string()))?;

            if !registry.is_model_ready(model_id).await {
                drop(registry_guard);

                tracing::info!(model_id = %model_id, "Downloading CLIP model on-demand");
                let registry_guard = self.model_registry.read().await;
                let registry = registry_guard
                    .as_ref()
                    .ok_or_else(|| Error::Backend("Model registry not initialized".to_string()))?;

                let path = registry
                    .download_model(model_id, None)
                    .await
                    .map_err(|e| Error::Backend(format!("Failed to download CLIP model: {}", e)))?;
                model_path = path;
            } else {
                model_path = registry.model_path(model_id);
            }
        }

        let device = raisin_ai::select_device(true)
            .map_err(|e| Error::Backend(format!("Failed to select device: {}", e)))?;
        tracing::info!(model_id = %model_id, device = ?device, "Loading CLIP embedder");

        let embedder = ClipEmbedder::with_model_id(&model_path, device, model_id.to_string())
            .map_err(|e| Error::Backend(format!("Failed to load CLIP embedder: {}", e)))?;

        let mut clip_guard = self.clip_embedder.write().await;
        *clip_guard = Some(embedder);

        Ok(())
    }

    /// Generate image embedding using CLIP
    async fn generate_image_embedding(&self, image_bytes: &[u8]) -> Result<Vec<f32>> {
        self.get_or_load_clip().await?;

        let clip_guard = self.clip_embedder.read().await;
        let embedder = clip_guard
            .as_ref()
            .ok_or_else(|| Error::Backend("CLIP embedder not loaded".to_string()))?;

        embedder
            .embed_image(image_bytes)
            .map_err(|e| Error::Backend(format!("CLIP embedding failed: {}", e)))
    }

    /// Handle an asset processing job
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let (node_id, options) = match &job.job_type {
            JobType::AssetProcessing { node_id, options } => (node_id.clone(), options.clone()),
            _ => {
                return Err(Error::Validation(format!(
                    "Expected AssetProcessing job, got: {}",
                    job.job_type
                )))
            }
        };

        tracing::info!(
            job_id = %job.id,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            node_id = %node_id,
            extract_pdf = options.extract_pdf_text,
            gen_img_embed = options.generate_image_embedding,
            "Processing asset"
        );

        let mut result = AssetProcessingResult {
            node_id: node_id.clone(),
            ..Default::default()
        };

        // Get the node from storage
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
                &node_id,
                Some(&context.revision),
            )
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node not found: {}", node_id)))?;

        // Get binary data if we have a callback
        // May HYDRATE a mount-owned asset that has no bytes yet, replacing
        // `node` with the copy that carries the stored file — the fingerprint
        // and mimetype below must come from the hydrated node, not the stub.
        let (node, binary_data) = self
            .retrieve_binary_data(job, &node_id, node, context)
            .await;

        let mime_type = extract_mime_type(&node);

        // THE EXTRACTION OUTCOME, produced for every path through this block —
        // including the ones that produce no text. `None` here means "nobody
        // asked for text from this asset"; every other case is a durable record.
        let mut artifact: Option<ExtractionArtifact> = None;
        let fingerprint = super::helpers::asset_fingerprint(&node);

        // Process based on mime type and options
        if let Some(ref data) = binary_data {
            // Text extraction. The mime vocabulary lives in `is_extractable_mime`
            // / `process_extractable`, NOT in a literal here — the enqueue side
            // reads the same one.
            if options.extract_pdf_text {
                match process_extractable(&mime_type, data, &options).await {
                    Some(Ok(pdf_result)) => {
                        artifact = Some(
                            ExtractionArtifact::extracted(
                                fingerprint.clone(),
                                pdf_result.source,
                                pdf_result.text.clone(),
                            )
                            .with_confidence(pdf_result.confidence, pdf_result.detail.clone()),
                        );
                        result.extracted_text = Some(pdf_result.text);
                        result.pdf_page_count = Some(pdf_result.page_count);
                        result.used_ocr = pdf_result.used_ocr;

                        tracing::info!(
                            job_id = %job.id,
                            node_id = %node_id,
                            page_count = pdf_result.page_count,
                            used_ocr = pdf_result.used_ocr,
                            confidence = ?pdf_result.confidence,
                            text_length = result.extracted_text.as_ref().map(|t| t.len()).unwrap_or(0),
                            "Text extraction complete"
                        );
                    }
                    Some(Err(e)) => {
                        // A retryable failure, recorded as one. It must NOT read
                        // as `unsupported`, or a backfill looking for
                        // newly-readable formats would sweep it up forever.
                        // The source is the EXTRACTOR that failed, and it is
                        // not always the PDF one now that images OCR through
                        // the same dispatch. A failure filed under the wrong
                        // extractor sends the next reader to the wrong code.
                        artifact = Some(ExtractionArtifact::failed(
                            fingerprint.clone(),
                            if is_image_mime(&mime_type) {
                                "core-image-ocr"
                            } else {
                                "core-pdf"
                            },
                            e.to_string(),
                        ));
                        tracing::error!(
                            job_id = %job.id, node_id = %node_id, error = %e,
                            "Text extraction failed"
                        );
                    }
                    None => {
                        artifact = Some(ExtractionArtifact::unsupported(
                            fingerprint.clone(),
                            unsupported_detail(&mime_type, &options),
                        ));
                        tracing::warn!(
                            job_id = %job.id, node_id = %node_id, mime_type = ?mime_type,
                            "Text extraction requested but no extractor handles this mime type \
                             (see is_extractable_mime / process_extractable)"
                        );
                    }
                }
            } else if options.text_delegated {
                // Text is coming, from a layer this job cannot reach. Not
                // `unsupported`: the plugin that reads this format is loaded
                // and has been handed the work, so claiming the bytes are
                // unreadable would both be false and enrol the asset in the
                // backfill that sweeps up formats a plugin has newly gained.
                artifact = Some(ExtractionArtifact::delegated(
                    fingerprint.clone(),
                    &options.delegated_tasks,
                ));
                tracing::info!(
                    job_id = %job.id, node_id = %node_id, mime_type = ?mime_type,
                    delegated = ?options.delegated_tasks,
                    "Text extraction is delegated to a plugin task above the job layer; \
                     recording __extract_status = 'delegated'"
                );
            } else if options.extract_text_requested {
                // THE CASE THIS WHOLE ARTIFACT EXISTS FOR. The routing table
                // asked for text and this process cannot produce it — a `.docx`
                // on a server with no media plugin. Before, `extract_pdf_text`
                // was already false here and the job wrote NOTHING: a node with
                // no text and no record that anything had been skipped,
                // indistinguishable from an empty document forever. Now it is
                // one row a backfill can find.
                artifact = Some(ExtractionArtifact::unsupported(
                    fingerprint.clone(),
                    unsupported_detail(&mime_type, &options),
                ));
                tracing::info!(
                    job_id = %job.id, node_id = %node_id, mime_type = ?mime_type,
                    blocked = ?options.blocked_tasks,
                    "Text was requested but nothing on this server can read these bytes; \
                     recording __extract_status = 'unsupported'"
                );
            }

            // Image processing (CLIP embeddings)
            if is_image_mime(&mime_type) && options.generate_image_embedding {
                self.process_image_embedding(job, &node_id, data, &mut result)
                    .await;
            }
        } else if options.extract_text_requested || options.text_delegated {
            // Asked for text and the bytes could not be read at all. Retryable,
            // so `failed` and not `unsupported` — a better extractor would not
            // help here.
            artifact = Some(ExtractionArtifact::failed(
                fingerprint.clone(),
                "none",
                "binary could not be retrieved from storage",
            ));
        }

        // PERSIST. Everything above only produced values in memory; until this
        // ran, the job serialised them into its result JSON and returned, so an
        // operator who switched on "extract PDF text" got a job that reported
        // success and changed nothing anywhere.
        if let Some(artifact) = artifact {
            self.persist_extraction_artifact(job, &node, &artifact, &options, &mut result, context)
                .await;
        }

        tracing::info!(
            job_id = %job.id,
            node_id = %node_id,
            has_extracted_text = result.extracted_text.is_some(),
            stored = result.extracted_text_stored,
            has_caption = result.caption.is_some(),
            has_keywords = result.keywords.is_some(),
            "Asset processing complete"
        );

        Ok(Some(serde_json::to_value(result).unwrap_or_default()))
    }

    /// Write the EXTRACTION ARTIFACT onto the node.
    ///
    /// # Why a node property, and why through `NodeService`
    ///
    /// A node property is the one shape every downstream index already
    /// understands. Writing it through the normal write path means the existing
    /// commit → `emit_node_events` → fulltext + `EmbeddingGenerate` funnel picks
    /// the text up with no new plumbing, replication carries it to peers for
    /// free (an arriving replica already HAS the text, so it never re-extracts —
    /// the apply path does not run this job), and the text is visible to SQL,
    /// `FULLTEXT_MATCH` and the vector search through the same surfaces as any
    /// other property.
    ///
    /// It is also what makes PUBLISH work without an extractor: Studio publish
    /// is a SQL copy of `node_type, archetype, properties, updated_at`, so an
    /// artifact in `properties` rides along to the publish branch and the
    /// embedding there is a pure embed call over text that is already present.
    /// That is why the artifact is INLINE and not a child node or a blob
    /// reference — see `raisin_models::nodes::extraction`.
    ///
    /// The alternative — an index writer of its own here, or teaching the
    /// synchronous embedding text walk to open files — would be a second
    /// extraction/indexing path beside this one, which is precisely the drift
    /// this codebase keeps paying for.
    ///
    /// `storage.nodes().update(...)` would NOT do: the raw repository emits no
    /// `node:updated`, so nothing downstream would ever see the text.
    ///
    /// # It writes on EVERY outcome
    ///
    /// Including `unsupported`, and including when `store_extracted_text` is
    /// off. The text is content and honours that setting; the status is
    /// BOOKKEEPING about the binary, and dropping it is what made a skip
    /// invisible in the first place.
    async fn persist_extraction_artifact(
        &self,
        job: &JobInfo,
        node: &raisin_models::nodes::Node,
        artifact: &ExtractionArtifact,
        options: &raisin_storage::jobs::AssetProcessingOptions,
        result: &mut AssetProcessingResult,
        context: &JobContext,
    ) {
        if !options.store_extracted_text {
            // Honour the option literally for the TEXT. `trigger_embedding`
            // cannot be honoured on its own: the embedding is generated from the
            // node's indexed properties, so with nothing stored there is nothing
            // to embed. The status record is written regardless.
            tracing::info!(
                job_id = %job.id, node_id = %node.id,
                trigger_embedding = options.trigger_embedding,
                "Extracted text discarded: store_extracted_text is off. \
                 Embedding needs a stored property, so trigger_embedding alone does nothing. \
                 The extraction STATUS is still recorded."
            );
        }

        // Written as a NAMED engine actor, not as plain `system`: the write-path
        // shield (`raisin_models::nodes::is_engine_write_actor`) opens only for
        // the two actors that own `__` properties, and `AuthContext::system()`
        // is what half the internal services and every test harness present.
        let svc: NodeService<RocksDBStorage> = NodeService::new_with_context(
            self.storage.clone(),
            context.tenant_id.clone(),
            context.repo_id.clone(),
            context.branch.clone(),
            context.workspace_id.clone(),
        )
        .with_auth(raisin_models::auth::AuthContext::system_as(
            raisin_models::nodes::EXTRACTION_ACTOR,
        ));

        // Re-read at HEAD rather than writing back the revision the job was
        // handed: extraction is seconds of work, and anything an editor changed
        // meanwhile must not be reverted by us.
        let fresh = match svc.get(&node.id).await {
            Ok(Some(n)) => n,
            Ok(None) => {
                tracing::warn!(
                    job_id = %job.id, node_id = %node.id,
                    "Node vanished before the extraction artifact could be stored"
                );
                return;
            }
            Err(e) => {
                tracing::error!(
                    job_id = %job.id, node_id = %node.id, error = %e,
                    "Failed to re-read node to store the extraction artifact"
                );
                return;
            }
        };

        let mut updated = fresh;
        // ONE writer of the artifact's property shape, in `raisin-models`, so a
        // partially-written artifact (a status with no fingerprint, which
        // re-extracts forever; a fingerprint with no status, which is the
        // invisible skip again) is not expressible here.
        artifact.apply(&mut updated.properties, options.store_extracted_text);

        match svc.update_node(updated).await {
            Ok(()) => {
                result.extracted_text_stored =
                    options.store_extracted_text && artifact.status == ExtractStatus::Ok;
                tracing::info!(
                    job_id = %job.id, node_id = %node.id,
                    status = %artifact.status.as_str(),
                    source = %artifact.source,
                    text_length = artifact.text.len(),
                    fingerprint = %artifact.fingerprint,
                    "Stored extraction artifact on node; fulltext + embedding follow from the node event"
                );
            }
            Err(e) => {
                tracing::error!(
                    job_id = %job.id, node_id = %node.id, error = %e,
                    "Failed to store the extraction artifact on node"
                );
            }
        }
    }

    /// Retrieve binary data from storage using the callback
    /// The asset's bytes, fetching them from the mount first when they are not
    /// stored yet.
    ///
    /// # Why hydration lives HERE
    ///
    /// A virtual mount syncs METADATA ONLY, so a mounted document arrives with
    /// no `file` at all. Every consumer of an asset's bytes starts from the
    /// storage key, so all of them failed identically on such a node — nothing
    /// was extracted, no thumbnail was made, and the file was findable only by
    /// its name. It was not broken; the bytes had simply never been asked for.
    ///
    /// This is the job that WANTS the bytes, and the only place that knows the
    /// processing plan, so it is where the decision to spend a download belongs.
    /// Doing it lazily in a viewer instead would index a document only once a
    /// human opened it, which is backwards for search.
    ///
    /// Returns the node it actually read, which is the HYDRATED one when a fetch
    /// happened: the fingerprint stamped by the caller has to describe the bytes
    /// that were read, and the stub's fingerprint describes nothing.
    async fn retrieve_binary_data(
        &self,
        job: &JobInfo,
        node_id: &str,
        node: raisin_models::nodes::Node,
        context: &JobContext,
    ) -> (raisin_models::nodes::Node, Option<Vec<u8>>) {
        let Some(callback) = self.binary_callback.as_ref() else {
            return (node, None);
        };

        // No stored bytes? It may simply never have been fetched. Always hands
        // a node back — the original when nothing could be done — so the caller
        // still records a durable outcome for it.
        let node = match extract_storage_key(&node) {
            Ok(_) => node,
            Err(_) => {
                self.hydrate_mounted_content(job, node_id, node, context)
                    .await
            }
        };

        let data = match extract_storage_key(&node) {
            Ok(storage_key) => match callback(storage_key.clone()).await {
                Ok(data) => {
                    tracing::debug!(
                        job_id = %job.id, node_id = %node_id,
                        storage_key = %storage_key, data_size = data.len(),
                        "Retrieved binary data"
                    );
                    Some(data)
                }
                Err(e) => {
                    tracing::warn!(
                        job_id = %job.id, node_id = %node_id,
                        storage_key = %storage_key, error = %e,
                        "Failed to retrieve binary data"
                    );
                    None
                }
            },
            Err(e) => {
                tracing::debug!(
                    job_id = %job.id, node_id = %node_id, error = %e,
                    "No storage key; nothing to read"
                );
                None
            }
        };

        (node, data)
    }

    /// Largest mounted file this will pull down to index.
    ///
    /// A bound, not a tuning knob: hydration exists so a synced drive can be
    /// SEARCHED, and past a certain size a file is not a document any more. It
    /// is checked against the size the provider already reported, so an
    /// oversized file costs no download at all — only the metadata we already
    /// hold.
    const MAX_HYDRATE_BYTES: u64 = 64 * 1024 * 1024;

    /// Fetch a mount-owned asset's bytes so it can be indexed.
    ///
    /// Returns the hydrated node, or the one it was given when nothing could be
    /// done — a caller must still be able to record an outcome.
    ///
    /// # What it deliberately does NOT do
    ///
    /// It does not fetch everything on the mount. The job only exists for an
    /// asset whose processing rules planned work for it, so the mimetype filter
    /// has already been applied by the time we get here: a drive full of video
    /// stays metadata-only, and its documents get read. The size cap is the
    /// second half of that bound.
    async fn hydrate_mounted_content(
        &self,
        job: &JobInfo,
        node_id: &str,
        node: raisin_models::nodes::Node,
        context: &JobContext,
    ) -> raisin_models::nodes::Node {
        if !raisin_models::nodes::is_fetchable_mount_content(&node.properties) {
            return node;
        }

        if let Some(size) = raisin_models::nodes::asset_reported_size(&node.properties) {
            if size > Self::MAX_HYDRATE_BYTES {
                tracing::info!(
                    job_id = %job.id, node_id = %node_id, size,
                    limit = Self::MAX_HYDRATE_BYTES,
                    "Not fetching mounted content: larger than the indexing limit"
                );
                return node;
            }
        }

        // Absent when the sync engine is not running in this process. That is a
        // deployment fact rather than a fault of this asset, so it is logged and
        // the asset is left alone rather than recorded as unreadable.
        let Some(mounts) = self.storage.virtual_mount_sync_handler() else {
            tracing::debug!(
                job_id = %job.id, node_id = %node_id,
                "Mounted content not fetched: the virtual-mount sync engine is not running"
            );
            return node;
        };

        // The two branches are NOT the same value: `branch` addresses the
        // materialized asset, `config_branch` is where the mount and integration
        // nodes live. Passing the target branch for both resolves no mount.
        //
        // Resolved the way the sync engine resolves it, NOT hardcoded to `main`.
        // That constant is what the integrations HTTP layer uses, and its own
        // comment records the consequence: on a repo whose default branch is not
        // `main` the two disagree and the engine finds no mount, returning a
        // silent success. `config_branch` is `pub(crate)` for exactly this
        // reason — so there is one answer to "which branch is a mount's config
        // on".
        let config_branch = crate::jobs::handlers::virtual_mount_sync::check::config_branch(
            &self.storage,
            &context.tenant_id,
            &context.repo_id,
        )
        .await;

        let target = crate::ContentTarget {
            tenant: &context.tenant_id,
            repo: &context.repo_id,
            config_branch: &config_branch,
            branch: &context.branch,
            workspace: &context.workspace_id,
            node_id,
        };

        match mounts.fetch_content(target, false).await {
            Ok(fetched) => {
                tracing::info!(
                    job_id = %job.id, node_id = %node_id, ?fetched,
                    "Fetched mounted content so the asset can be indexed"
                );
            }
            Err(e) => {
                // Not fatal. The asset keeps no `file`, the caller records the
                // outcome it would have anyway, and the next sync or an explicit
                // retry can try again.
                tracing::warn!(
                    job_id = %job.id, node_id = %node_id, error = %e,
                    "Could not fetch mounted content"
                );
                return node;
            }
        }

        // Re-read: `fetch_content` wrote the `file` onto the stored node, and
        // everything downstream — the fingerprint especially — must describe the
        // bytes that were actually fetched.
        match self
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
                None,
            )
            .await
        {
            Ok(Some(fresh)) => fresh,
            _ => node,
        }
    }

    /// Process image embedding using CLIP
    async fn process_image_embedding(
        &self,
        job: &JobInfo,
        node_id: &str,
        data: &[u8],
        result: &mut AssetProcessingResult,
    ) {
        tracing::info!(
            job_id = %job.id, node_id = %node_id,
            "Generating image embedding with CLIP"
        );

        match self.generate_image_embedding(data).await {
            Ok(embedding) => {
                result.image_embedding_generated = true;
                result.image_embedding_dim = Some(embedding.len());
                result.image_embedding = Some(embedding);

                tracing::info!(
                    job_id = %job.id, node_id = %node_id,
                    embedding_dim = result.image_embedding_dim,
                    "Image embedding generated successfully"
                );
            }
            Err(e) => {
                tracing::error!(
                    job_id = %job.id, node_id = %node_id, error = %e,
                    "Failed to generate image embedding"
                );
            }
        }
    }
}

/// One sentence naming WHY nothing could read these bytes.
///
/// It is stored on the node, not only logged, because the log line an operator
/// would have to correlate it with is hours old by the time anyone asks. The
/// blocked-task list comes from the routing plan, so a `.docx` on a server with
/// no media plugin says so by name instead of saying "unsupported".
fn unsupported_detail(
    mime_type: &Option<String>,
    options: &raisin_storage::jobs::AssetProcessingOptions,
) -> String {
    let mime = mime_type.as_deref().unwrap_or("unknown");
    if options.blocked_tasks.is_empty() {
        format!("no extractor on this server handles {mime}")
    } else {
        format!(
            "no extractor on this server handles {mime}; blocked tasks: {}",
            options.blocked_tasks.join(", ")
        )
    }
}
