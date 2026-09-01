//! Asset processing logic for the event handler
//!
//! Handles extraction of file metadata (storage keys, content hashes, MIME types)
//! from node properties, and builds processing options based on configured rules.

use super::UnifiedJobEventHandler;
use crate::jobs::handlers::asset_processing::helpers as asset_helpers;
use crate::jobs::handlers::asset_processing::{
    EXTRACTION_ARTIFACT_VERSION, EXTRACTION_FINGERPRINT_PROP, EXTRACT_VERSION_PROP,
};
use raisin_ai::RuleMatchContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::jobs::{AssetProcessingOptions, JobContext, PdfExtractionStrategy};
use raisin_storage::ProcessingRulesRepository;

impl UnifiedJobEventHandler {
    /// Check if an asset node should be processed for AI features
    ///
    /// Returns true if:
    /// 1. Node type is "raisin:Asset"
    /// 2. File property exists with a storage_key (file is ready)
    /// 3. The current binary has not already been extracted (see below)
    ///
    /// # The fingerprint gate is what makes extraction terminate
    ///
    /// Extraction now lands in a node property, and writing a node property
    /// emits `node:updated` — the same event that gets us here. Without a gate,
    /// one upload would extract, write, extract, write, forever, each round
    /// minting a node revision, a fulltext reindex and an embedding call.
    ///
    /// This is the SINGLE gate for both enqueue call sites
    /// (`enqueue_asset_processing_if_needed` is reached from the metadata path
    /// and from the fetch-from-storage path), so it cannot be half-applied.
    ///
    /// # Arguments
    /// * `node` - The node to check (required for file property inspection)
    pub(crate) fn should_process_asset(&self, node: &raisin_models::nodes::Node) -> bool {
        // Only process raisin:Asset nodes
        if node.node_type != "raisin:Asset" {
            return false;
        }

        // Check if file property exists and has storage_key (file is ready)
        let has_storage_key = self.extract_storage_key(node).is_some();

        if !has_storage_key {
            tracing::debug!(
                node_id = %node.id,
                "Skipping asset processing: no storage_key (file not ready)"
            );
            return false;
        }

        // Already extracted from exactly these bytes, BY THIS PIPELINE? Then
        // this event is our own write-back (or a replicated copy of it), and
        // there is nothing to redo. Replacing the file changes the fingerprint
        // and re-opens the gate.
        //
        // The VERSION is checked alongside the fingerprint, and that is what
        // makes a backfill possible at all. A stored `unsupported` is a durable
        // record that these bytes were offered to the extractors and none of
        // them could read them; the fingerprint alone would then close the gate
        // FOREVER, so the day a media plugin gains the format the asset would
        // still never be reconsidered. Bumping
        // `EXTRACTION_ARTIFACT_VERSION` re-opens every asset exactly once —
        // which is the operation "we can read more formats now" actually needs.
        if let Some(PropertyValue::String(stamped)) =
            node.properties.get(EXTRACTION_FINGERPRINT_PROP)
        {
            let current = asset_helpers::asset_fingerprint(node);
            let stored_version = match node.properties.get(EXTRACT_VERSION_PROP) {
                Some(PropertyValue::Integer(v)) => *v,
                // An artifact written before the version existed. Treat it as
                // generation 0, i.e. superseded, so it is redone once and then
                // carries a version like everything else.
                _ => 0,
            };
            if *stamped == current && stored_version >= EXTRACTION_ARTIFACT_VERSION {
                tracing::debug!(
                    node_id = %node.id,
                    fingerprint = %current,
                    version = stored_version,
                    "Skipping asset processing: this binary has already been extracted"
                );
                return false;
            }
        }

        true
    }

    /// Extract storage_key from a node's file property.
    ///
    /// Delegates to the ONE set of asset accessors, shared with the job
    /// handler. This used to be a second, narrower copy: it missed a storage
    /// key sitting directly on an `Object` `file` under `storageKey`, and a
    /// mime type in `contentType`, so an asset the handler could have processed
    /// was never enqueued in the first place — a gap visible nowhere, because
    /// both halves "worked".
    pub(crate) fn extract_storage_key(&self, node: &raisin_models::nodes::Node) -> Option<String> {
        asset_helpers::extract_storage_key(node).ok()
    }

    /// Extract content_hash from a node's file property. See
    /// [`Self::extract_storage_key`] for why this delegates.
    pub(crate) fn extract_content_hash(&self, node: &raisin_models::nodes::Node) -> Option<String> {
        asset_helpers::extract_content_hash(node)
    }

    /// Extract MIME type from a node's file property. See
    /// [`Self::extract_storage_key`] for why this delegates.
    pub(crate) fn extract_mime_type(&self, node: &raisin_models::nodes::Node) -> Option<String> {
        asset_helpers::extract_mime_type(node)
    }

    /// Fill unset OCR settings from the tenant's configured defaults.
    ///
    /// This is the "Rule setting > Tenant default > System default" order that
    /// `ProcessingDefaults` has documented since it was written and that nothing
    /// implemented: this function is reached per asset event and never loaded a
    /// tenant config at all, so a tenant-level setting was persisted, shown, and
    /// ignored.
    ///
    /// # Why the short-circuit is not an optimisation
    ///
    /// The read is skipped entirely when the rule already answers both
    /// questions, because this runs on every asset event and an unconditional
    /// config read would put a storage round-trip on that path for the common
    /// case where it can change nothing. The same guard, for the same reason, is
    /// documented on the embedding provider's AI-config read.
    ///
    /// A missing or unreadable tenant config is NOT an error — it means "no
    /// tenant defaults", which is the overwhelmingly common case and must not
    /// fail an upload.
    async fn apply_tenant_ocr_defaults(
        &self,
        mut settings: raisin_ai::ProcessingSettings,
        tenant_id: &str,
    ) -> raisin_ai::ProcessingSettings {
        if settings.ocr_languages.is_some() && settings.ocr_min_word_confidence.is_some() {
            return settings;
        }

        use raisin_ai::storage::TenantAIConfigStore;
        match self
            .storage
            .tenant_ai_config_repository()
            .get_config(tenant_id)
            .await
        {
            Ok(config) => {
                if let Some(defaults) = &config.processing_defaults {
                    defaults.apply_ocr_defaults(&mut settings);
                }
            }
            // A tenant with no AI config is the common case, not a fault: it
            // simply has no defaults to contribute.
            Err(raisin_ai::storage::StorageError::NotFound(_)) => {}
            Err(e) => {
                tracing::debug!(
                    tenant_id = %tenant_id,
                    error = %e,
                    "Could not read tenant AI config for OCR defaults; using rule settings alone"
                );
            }
        }
        settings
    }

    /// Build asset processing options by ROUTING the node's mimetype to a set
    /// of tasks, then projecting that task list onto the options this job
    /// understands.
    ///
    /// # Why the booleans are derived and no longer decided here
    ///
    /// This function used to make the routing decision twice: once inside the
    /// matched-rule branch and once in a mime-defaults fallback below it, each
    /// spelling out "PDF means extract, `image/*` means embed and caption" in
    /// its own literals. That is the mirrored-path drift CLAUDE.md names as this
    /// repo's #1 recurring bug class, and it had already produced one: the
    /// shipped `image-default` rule matched the literal string `"image/"`
    /// against an exact-equality matcher, so it selected nothing, every image
    /// fell through to the catch-all, and the fallback's `is_image` defaults
    /// were what actually ran. Two descriptions, one of them dead, and no error
    /// anywhere.
    ///
    /// Now `ProcessingSettings::effective_tasks` is the single derivation — an
    /// explicit `tasks` list if the rule has one, otherwise the legacy booleans
    /// projected onto the same vocabulary using the same mime defaults — and
    /// this function only asks the planner which of those tasks THIS process can
    /// run.
    ///
    /// # `|_| false` is not a placeholder
    ///
    /// The availability predicate says: no plugin method is reachable from here.
    /// That is the truth and it is structural, not a TODO. `raisin-functions`
    /// (which owns the plugin registry) depends on `raisin-rocksdb`, and Cargo
    /// rejects package cycles regardless of features — so this crate can never
    /// call `media.doc.toMarkdown`. A rule naming a plugin task is reported
    /// `blocked` here and has to be executed from the function/flow layer.
    /// Answering `true` would enqueue a job that silently does nothing, which is
    /// the exact failure this whole path was fixed to stop producing.
    pub(crate) async fn get_asset_processing_options(
        &self,
        node: &raisin_models::nodes::Node,
        context: &JobContext,
    ) -> AssetProcessingOptions {
        let mime_type = self.extract_mime_type(node);
        let content_hash = self.extract_content_hash(node);
        let mime = mime_type.as_deref().unwrap_or("");

        // Resolve the matching rule's settings, or the defaults. `get_settings`
        // is first-match-wins over the persisted rule set; a repo with no rules
        // configured yields `ProcessingSettings::default()`, whose
        // `effective_tasks` reproduces the old mime defaults exactly.
        let settings = match self
            .processing_rules
            .get_rules(raisin_storage::RepoScope::new(
                &context.tenant_id,
                &context.repo_id,
            ))
            .await
        {
            Ok(Some(rule_set)) => {
                let match_context = RuleMatchContext::new()
                    .with_node_type(&node.node_type)
                    .with_path(&node.path)
                    .with_mime_type(mime)
                    .with_workspace(&context.workspace_id);

                match rule_set.find_matching_rule(&match_context) {
                    Some(rule) => {
                        tracing::debug!(
                            rule_id = %rule.id,
                            rule_name = %rule.name,
                            node_path = %node.path,
                            mime_type = %mime,
                            "Matched processing rule"
                        );
                        rule.settings.clone()
                    }
                    None => raisin_ai::ProcessingSettings::default(),
                }
            }
            _ => raisin_ai::ProcessingSettings::default(),
        };

        let settings = self
            .apply_tenant_ocr_defaults(settings, &context.tenant_id)
            .await;

        let requested = settings.effective_tasks(mime_type.as_deref());
        // Against what this process can ACTUALLY do, not against `|_| false`.
        //
        // The hard-coded `false` was honest for a stock build and wrong for one
        // that had loaded a plugin: `plan_tasks` reported every delegated task
        // `plugin_missing` on a server that could perform it, so a `.docx` rule
        // was permanently dead however the operator configured it. The probe is
        // installed process-wide from `main.rs` after plugins load — see
        // `raisin_ai::rules::capabilities` for why it has to arrive from above.
        let plan = raisin_ai::plan_tasks_here(&requested);

        // `runnable` now means "will run", not "will run HERE". Split it: this
        // job performs the native half itself, and the delegated half belongs
        // to the function layer, which is the only place a plugin can be
        // called from. Collapsing the two is how a delegated task would end up
        // silently expected of a job that cannot do it.
        let (native, delegated): (Vec<_>, Vec<_>) =
            plan.runnable.iter().partition(|t| t.via.is_none());
        let delegated_tasks: Vec<String> = delegated
            .iter()
            .map(|t| match &t.via {
                Some(method) => format!("{} ({})", t.slug, method),
                None => t.slug.clone(),
            })
            .collect();

        // A task an operator configured that cannot run HERE is worth a line.
        // The silence this replaces is the reported symptom: uploading a .docx
        // produced one debug message and no record that anything was expected.
        if !plan.blocked.is_empty() {
            tracing::info!(
                node_path = %node.path,
                mime_type = %mime,
                blocked = ?plan.blocked,
                "Configured tasks cannot run in the AssetProcessing job; they need \
                 the function/flow layer (see raisin_ai::rules::tasks)"
            );
        }

        // "Was text asked for" and "can this process produce it" are now two
        // answers, not one. The first is the routing table's; the second is
        // this binary's extractor vocabulary. Collapsing them into a single
        // boolean is what made a skipped `.docx` invisible: the handler was
        // told nobody had asked, so it wrote no record at all. See
        // `AssetProcessingOptions::extract_text_requested`.
        //
        // A text task that is DELEGATED is a third answer and must not fold
        // into either: the routing table asked for text, this binary cannot
        // produce it, and yet `unsupported` would be a lie — the plugin that
        // can is loaded and has been handed the job. Recording it as
        // unsupported would also make the format-backfill sweep re-extract
        // assets whose conversion is running or already done.
        let text_delegated = delegated.iter().any(|t| is_text_bearing_task(&t.slug));
        let text_requested = plan.will_run("extract_text")
            || plan.blocked.iter().any(|b| is_text_bearing_task(&b.slug));

        // Carried into the job so the durable artifact can NAME the reason,
        // rather than recording a bare "unsupported" an operator has to
        // correlate with a log line.
        let blocked_tasks: Vec<String> = plan
            .blocked
            .iter()
            .map(|b| match &b.blocked {
                raisin_ai::BlockedReason::PluginMissing { method } => {
                    format!("{} (needs plugin method {})", b.slug, method)
                }
                raisin_ai::BlockedReason::MalformedSlug => {
                    format!("{} (malformed task slug)", b.slug)
                }
                // NOT an error and NOT a typo — a legible rule aimed one layer
                // down. Recording it verbatim means the asset's durable
                // artifact tells whoever asks what actually performs the task,
                // instead of leaving them to discover on their own that
                // "captioning is on" meant nothing.
                raisin_ai::BlockedReason::HandledAbove { how } => {
                    format!("{} (handled above this layer: {})", b.slug, how)
                }
                raisin_ai::BlockedReason::Unknown => {
                    format!("{} (no provider on this server)", b.slug)
                }
            })
            .collect();

        AssetProcessingOptions {
            // Projection of the plan, not a second decision. `extract_text` is
            // additionally gated on the extractor vocabulary — a rule may ask
            // for it on a mimetype nothing here can read, and the handler says
            // so rather than reporting a silent success.
            extract_pdf_text: plan.will_run("extract_text")
                && asset_helpers::is_extractable_mime(&mime_type),
            extract_text_requested: text_requested,
            text_delegated,
            blocked_tasks,
            delegated_tasks,
            generate_image_embedding: plan.will_run("image_embedding"),
            // The NATIVE half only. This field is what the handler dispatches
            // from; listing a plugin task here would tell it to perform work it
            // has no way to reach.
            tasks: native.iter().map(|t| t.slug.clone()).collect(),
            pdf_strategy: settings
                .pdf_strategy
                .map(|s| match s {
                    raisin_ai::PdfStrategy::Auto => PdfExtractionStrategy::Auto,
                    raisin_ai::PdfStrategy::NativeOnly => PdfExtractionStrategy::NativeOnly,
                    raisin_ai::PdfStrategy::OcrOnly => PdfExtractionStrategy::OcrOnly,
                    raisin_ai::PdfStrategy::ForceOcr => PdfExtractionStrategy::ForceOcr,
                })
                .unwrap_or_default(),
            store_extracted_text: settings.store_extracted_text.unwrap_or(true),
            trigger_embedding: settings.trigger_embedding.unwrap_or(true),
            content_hash,
            embedding_model: settings.embedding_model.clone(),
            ocr_languages: settings.ocr_languages.clone().unwrap_or_default(),
            // Defaulted HERE rather than left as `None`, because `None` reaches
            // the extractor as "keep every word" — so an unconfigured server
            // would store the noise this filter exists to remove. A rule that
            // genuinely wants everything says `0`.
            ocr_min_word_confidence: Some(
                settings
                    .ocr_min_word_confidence
                    .unwrap_or(raisin_ai::rules::DEFAULT_OCR_MIN_WORD_CONFIDENCE),
            ),
        }
    }
}

/// Task slugs whose OUTPUT is text for the extraction artifact.
///
/// One list, read by both the "was text asked for" and the "is text delegated"
/// questions. They used to be one inline `matches!` answering only the first;
/// a second copy for the second question is precisely the mirrored predicate
/// that drifts.
fn is_text_bearing_task(slug: &str) -> bool {
    matches!(
        slug,
        "extract_text" | "doc_to_markdown" | "doc_to_pdf" | "image_ocr"
    )
}
