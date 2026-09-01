//! Processing settings for content handling rules.

use serde::{Deserialize, Serialize};

use crate::config::ChunkingConfig;
use crate::pdf::PdfStrategy;

/// Settings that control how content is processed.
///
/// All fields are optional - if None, the default or tenant-level setting is used.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessingSettings {
    /// The tasks to run on a node this rule matches, in order.
    ///
    /// # This is the "set of tasks" half of the routing table
    ///
    /// A rule already says WHICH nodes (`matcher`, which can now name a
    /// mimetype family). This says WHAT HAPPENS to them, as an open list of
    /// slugs — see [`crate::rules::tasks`] for the vocabulary and why it is
    /// open rather than an enum.
    ///
    /// `None` means "not configured", NOT "no tasks". A rule set written before
    /// this field existed still has to behave exactly as it did, so
    /// [`ProcessingSettings::effective_tasks`] falls back to deriving the list
    /// from the legacy booleans below. Setting it to `Some(vec![])` is the way
    /// to say "match these nodes and do nothing" — which is a real
    /// configuration (an opt-out rule ordered ahead of a broad one), and used
    /// to be inexpressible.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tasks: Option<Vec<String>>,

    /// Chunking configuration for text embedding.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        deserialize_with = "deserialize_chunking_compat"
    )]
    pub chunking: Option<ChunkingConfig>,

    /// PDF extraction strategy.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pdf_strategy: Option<PdfStrategy>,
    /// Whether to generate image embeddings (CLIP).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub generate_image_embedding: Option<bool>,

    // The six captioning knobs that used to live here — `generate_image_caption`,
    // `caption_model`, `alt_text_prompt`, `description_prompt`,
    // `generate_keywords`, `keywords_prompt` — are GONE, deliberately and
    // completely: from this struct, from `AssetProcessingOptions`, from the job
    // handler and from the admin console, in one move.
    //
    // Nothing read them. `AssetProcessingHandler` answered all six with a
    // single `tracing::warn!` saying captioning is disabled and to use a
    // trigger function instead. The console still rendered six controls that
    // wrote settings no code consulted, which is worse than having no feature:
    // an operator could turn captioning "on", see it persisted, and get
    // nothing, with the only evidence a warn line in a log they were not
    // reading.
    //
    // Removing them from the UI alone would have left the settings still
    // deserializing off stored rules and still serializing into every job
    // payload — a third state, invisible from both ends. So they are removed
    // everywhere at once. `image_caption` and `image_keywords` survive as TASK
    // SLUGS (see `crate::rules::tasks`), now honestly classified as
    // `TaskProvider::Function`, so a rule that names one gets told where the
    // work actually happens instead of a green plan and silence.
    //
    // Serde ignores unknown fields, so a rule stored with the old keys still
    // loads; the keys are simply dropped on the next write.
    /// Model to use for embeddings.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub embedding_model: Option<String>,

    /// Whether to trigger embedding generation after text extraction.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trigger_embedding: Option<bool>,

    /// Whether to store extracted text in node properties.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub store_extracted_text: Option<bool>,

    /// Maximum text length to store (for extracted text).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_stored_text_length: Option<usize>,

    /// Languages OCR should recognise, as **Tesseract** codes (`eng`, `deu`),
    /// not ISO 639-1 (`en`, `de`) — see [`crate::pdf::ocr::OcrOptions`].
    ///
    /// Tesseract reads all of them in ONE pass, so a mixed-language page needs
    /// no detection stage. The cost is memory: each language is roughly 10 MB
    /// resident and ~0.06s per image, and asking for all 112 installed measured
    /// 1.18 GB and 4.5s — which several concurrent asset jobs cannot afford.
    /// That is why this is a short list per rule and not "everything".
    ///
    /// A deployment spanning borders expresses it as ordinary narrower rules:
    /// a `Path` matcher on `/drives/uk/**` selecting `["eng"]` ahead of a
    /// broader `image/*` rule selecting the local set.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ocr_languages: Option<Vec<String>>,

    /// Drop OCR words scored below this (0-100). `None` keeps everything.
    ///
    /// Tesseract scores each word, and on a photograph the gap is stark: a real
    /// sign read 81-95 while two tokens invented from texture read 27. A page
    /// average cannot express that, which is why the filter is per word.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ocr_min_word_confidence: Option<u8>,
}

/// Per-word confidence floor applied when a rule does not name one.
///
/// Measured: a clean document scores 94-96 per word, while the tokens an engine
/// invents from photographic texture score below 30. 50 sits in the empty space
/// between, so it discards noise without touching a legible scan. A rule wanting
/// no filtering at all says `0` explicitly.
pub const DEFAULT_OCR_MIN_WORD_CONFIDENCE: u8 = 50;

/// The most languages one rule may ask for.
///
/// Not a taste limit — a memory one. Each traineddata is ~10 MB resident and
/// asset jobs run concurrently, so an unbounded list lets one rule edit OOM the
/// worker for every tenant on the box. Eight covers any realistic document mix.
pub const MAX_OCR_LANGUAGES: usize = 8;

impl ProcessingSettings {
    /// Check the OCR settings are ones Tesseract can actually be given.
    ///
    /// Called on write rather than on read: a rule that would OOM the asset
    /// worker should be refused by the person editing it, who can see the error,
    /// not discovered later by a background job that can only log.
    pub fn validate_ocr(&self) -> Result<(), String> {
        // Checked FIRST: the language early-return below must not skip it, or a
        // rule setting only a confidence goes unvalidated.
        if let Some(c) = self.ocr_min_word_confidence {
            if c > 100 {
                return Err(format!("ocr_min_word_confidence is 0-100, got {c}"));
            }
        }

        let Some(langs) = &self.ocr_languages else {
            return Ok(());
        };
        if langs.is_empty() {
            return Err("ocr_languages is empty; omit it to use the default".to_string());
        }
        if langs.len() > MAX_OCR_LANGUAGES {
            return Err(format!(
                "{} OCR languages requested, at most {MAX_OCR_LANGUAGES} are allowed: \
                 each costs about 10 MB of resident model and asset jobs run concurrently",
                langs.len()
            ));
        }
        // Shape only — the installed set differs per machine, so membership is
        // not knowable here. This catches the ISO 639-1 slip (`de` for `deu`),
        // which is the mistake the old doc comment actively encouraged.
        for l in langs {
            if l.len() < 3 || !l.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!(
                    "'{l}' is not a Tesseract language code. They are three or more \
                     alphanumerics (eng, deu, chi_sim), not ISO 639-1 (en, de)"
                ));
            }
        }
        Ok(())
    }
}

/// Helper enum for backward compatibility with boolean chunking config
#[derive(Deserialize)]
#[serde(untagged)]
enum ChunkingConfigCompat {
    Bool(bool),
    Config(ChunkingConfig),
}

fn deserialize_chunking_compat<'de, D>(deserializer: D) -> Result<Option<ChunkingConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let helper: Option<ChunkingConfigCompat> = Option::deserialize(deserializer)?;
    match helper {
        Some(ChunkingConfigCompat::Bool(true)) => Ok(Some(ChunkingConfig::default())),
        Some(ChunkingConfigCompat::Bool(false)) => Ok(None),
        Some(ChunkingConfigCompat::Config(c)) => Ok(Some(c)),
        None => Ok(None),
    }
}

impl ProcessingSettings {
    /// Create settings for PDF processing.
    pub fn pdf() -> Self {
        Self {
            tasks: Some(vec!["extract_text".to_string()]),
            pdf_strategy: Some(PdfStrategy::Auto),
            trigger_embedding: Some(true),
            store_extracted_text: Some(true),
            ..Default::default()
        }
    }

    /// Create settings for image processing.
    pub fn image() -> Self {
        Self {
            // `image_caption` is NOT in the built-in image default any more. It
            // is a product-layer task now, so listing it here would put a
            // permanently-blocked entry in the plan of every image upload on
            // every server — noise that says nothing, on the most common path
            // there is. A tenant that wants it says so explicitly.
            //
            // `extract_text` (OCR) IS here, and it has to be spelled out: an
            // explicit `tasks` list wins over the mime defaults in
            // `effective_tasks`, so widening those alone would have left the
            // built-in image rule — the one nearly every image actually
            // matches — silently opted out of the feature.
            tasks: Some(vec![
                "extract_text".to_string(),
                "image_embedding".to_string(),
            ]),
            generate_image_embedding: Some(true),
            ..Default::default()
        }
    }

    /// The task list this rule actually asks for, given the node's mime type.
    ///
    /// # One derivation, not two
    ///
    /// The acting side needs a single input. Before, it read four booleans, and
    /// the defaulting for those booleans ("PDF means extract, `image/*` means
    /// embed and caption") lived in the job event handler, spelled out inline —
    /// so the rule set and the mime defaults were two independent descriptions
    /// of the same decision. Adding a task list beside them would have made
    /// three.
    ///
    /// So this is the ONE place that answers "what should happen to this node",
    /// and the booleans are read only here. An explicit `tasks` wins outright;
    /// otherwise the legacy booleans are projected onto the same vocabulary,
    /// using `mime_type` for exactly the defaults the event handler used to
    /// apply. Callers that want the boolean shape should derive it from the
    /// result rather than reading the fields again.
    ///
    /// `mime_type` is `None` for a node that is not an asset — a plain content
    /// node with no bytes. Nothing mime-defaulted applies to it, which is
    /// correct: it has no binary to open.
    pub fn effective_tasks(&self, mime_type: Option<&str>) -> Vec<String> {
        if let Some(explicit) = &self.tasks {
            return explicit.clone();
        }

        let is_image = mime_type
            .map(|m| crate::rules::mime_matches("image/*", m))
            .unwrap_or(false);
        // DELIBERATELY WIDER than what this binary can read.
        //
        // It used to mirror `is_extractable_mime` in `raisin-rocksdb` (PDF and
        // nothing else) so that a planned task could never silently do nothing.
        // That conflated two different questions and lost the answer to the
        // important one: a `.docx` produced NO `extract_text` task, so the job
        // was told nobody had asked, so it wrote nothing — and the asset became
        // indistinguishable from an empty document, with nothing to query when
        // a media plugin later gained the format.
        //
        // The routing table's job is "what SHOULD happen to this binary".
        // Whether it CAN happen here is `plan_tasks`' job, and what to record
        // when it cannot is the handler's (`__extract_status = 'unsupported'`).
        // Keeping the three separate is what makes a skip durable and findable
        // instead of silent.
        let is_extractable = mime_type.map(is_text_bearing).unwrap_or(false);

        let mut tasks = Vec::new();
        // An image is text-bearing too, just not in the MIME sense: OCR reads
        // the scanned invoice, the whiteboard photo and the screenshot of an
        // error message, none of which the index could otherwise see past the
        // filename. `is_text_bearing` stays honest about MIME semantics and the
        // OR happens here, where the question is "what should be attempted".
        //
        // Most images yield nothing, which the handler records as `ok` with
        // zero characters — durable, cheap to skip, never retried.
        if is_extractable || is_image {
            tasks.push("extract_text".to_string());
        }
        if self.generate_image_embedding.unwrap_or(is_image) {
            tasks.push("image_embedding".to_string());
        }
        // No `image_caption` / `image_keywords` default. An image used to
        // derive both, which meant every image upload planned two tasks that
        // this process has never performed. Defaulting a task nobody runs is
        // how the whole surface stayed invisible: the plan said yes, so nothing
        // ever looked closer.
        tasks
    }

    /// Merge with another settings, preferring values from `other`.
    pub fn merge(&self, other: &ProcessingSettings) -> ProcessingSettings {
        ProcessingSettings {
            tasks: other.tasks.clone().or_else(|| self.tasks.clone()),
            chunking: other.chunking.clone().or_else(|| self.chunking.clone()),
            pdf_strategy: other.pdf_strategy.or(self.pdf_strategy),
            generate_image_embedding: other
                .generate_image_embedding
                .or(self.generate_image_embedding),
            embedding_model: other
                .embedding_model
                .clone()
                .or_else(|| self.embedding_model.clone()),
            trigger_embedding: other.trigger_embedding.or(self.trigger_embedding),
            store_extracted_text: other.store_extracted_text.or(self.store_extracted_text),
            max_stored_text_length: other.max_stored_text_length.or(self.max_stored_text_length),
            ocr_languages: other
                .ocr_languages
                .clone()
                .or_else(|| self.ocr_languages.clone()),
            ocr_min_word_confidence: other
                .ocr_min_word_confidence
                .or(self.ocr_min_word_confidence),
        }
    }
}

/// Mimetypes a text extractor SHOULD be asked about.
///
/// Membership means "this binary plausibly contains text a human would want
/// searched", never "this build can read it" — see the comment in
/// [`ProcessingSettings::effective_tasks`] for why those must stay different
/// questions. A type listed here that nothing can read produces a durable
/// `unsupported` record; a type NOT listed here produces no record at all, so
/// the list is the boundary between "we know we skipped this" and "we never
/// considered it".
///
/// Office types are matched by their full `officedocument`/`opendocument`
/// names AND by the older `application/msword` family, because uploads carry
/// both spellings depending on the client.
pub fn is_text_bearing(mime: &str) -> bool {
    let mime = mime.split(';').next().unwrap_or(mime).trim();
    if mime.is_empty() {
        return false;
    }
    if crate::rules::mime_matches("text/*", mime) {
        return true;
    }
    matches!(
        mime.to_ascii_lowercase().as_str(),
        "application/pdf"
            | "application/rtf"
            | "application/epub+zip"
            | "application/json"
            | "application/xml"
            | "application/msword"
            | "application/vnd.ms-excel"
            | "application/vnd.ms-powerpoint"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            | "application/vnd.oasis.opendocument.text"
            | "application/vnd.oasis.opendocument.spreadsheet"
            | "application/vnd.oasis.opendocument.presentation"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_tasks_win_over_every_legacy_boolean() {
        let s = ProcessingSettings {
            tasks: Some(vec!["doc_to_markdown".into()]),
            generate_image_embedding: Some(true),
            ..Default::default()
        };
        assert_eq!(
            s.effective_tasks(Some("image/png")),
            vec!["doc_to_markdown"]
        );
    }

    #[test]
    fn an_empty_task_list_is_a_real_opt_out() {
        // Distinct from `None`. A rule ordered ahead of a broad one, matching
        // `/scratch/**`, saying "do nothing to these" — previously
        // inexpressible, because absent config meant mime defaults.
        let s = ProcessingSettings {
            tasks: Some(vec![]),
            ..Default::default()
        };
        assert!(s.effective_tasks(Some("application/pdf")).is_empty());
    }

    #[test]
    fn legacy_settings_keep_their_old_mime_defaults() {
        let s = ProcessingSettings::default();
        assert_eq!(
            s.effective_tasks(Some("application/pdf")),
            vec!["extract_text"]
        );
        // `extract_text` (OCR) and `image_embedding` — and still NOT
        // `image_caption` / `image_keywords`. An image used to derive those two
        // here as well, which meant every image upload on every server planned
        // two tasks that no code in this process has ever performed. That is
        // how the dead captioning surface stayed invisible: the plan said yes.
        //
        // OCR is different in kind: `process_image_ocr` really does run, and
        // the text it finds is what makes a scanned invoice findable by its
        // contents rather than by its filename.
        assert_eq!(
            s.effective_tasks(Some("image/png")),
            vec!["extract_text", "image_embedding"]
        );
        // A DOCUMENT is asked for text even when this build cannot read it.
        //
        // This is the reversal that makes a skip visible. Before, a `.docx`
        // produced no task, so the asset job was told nobody had asked and
        // wrote nothing — no text, and no record that anything had been
        // skipped, which is indistinguishable from an empty document forever.
        // The task is now planned, `plan_tasks` reports what can run, and the
        // handler records `__extract_status = 'unsupported'` when nothing can.
        assert_eq!(
            s.effective_tasks(Some(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            )),
            vec!["extract_text"]
        );

        // A binary that carries no text at all is still asked for nothing —
        // the boundary is "we know we skipped this" vs "we never considered
        // it", and an mp4 belongs on the far side of it.
        assert!(s.effective_tasks(Some("video/mp4")).is_empty());
    }

    #[test]
    fn an_image_is_asked_for_text_even_though_its_mime_is_not_text_bearing() {
        // The two questions are separate and both answers are correct:
        // `is_text_bearing` describes the MIME type, where an image carries no
        // text; `effective_tasks` describes what should be ATTEMPTED, where OCR
        // is exactly the thing that reads a screenshot or a scanned page.
        assert!(!is_text_bearing("image/png"));

        let planned = ProcessingSettings::default().effective_tasks(Some("image/png"));
        assert!(
            planned.contains(&"extract_text".to_string()),
            "an image must plan OCR, or a scanned document is findable only by \
             its filename; got {planned:?}"
        );

        // And it must not have widened anything else on the way past: a format
        // nothing can read still plans nothing, which is what keeps
        // `__extract_status = 'unsupported'` meaningful.
        assert!(ProcessingSettings::default()
            .effective_tasks(Some("video/mp4"))
            .is_empty());
    }

    #[test]
    #[test]
    fn merge_carries_the_ocr_settings_both_ways() {
        // `merge` builds an exhaustive struct literal with no `..`, so a field
        // missing from it is a compile error rather than a silent drop. This
        // pins the PRECEDENCE instead: `other` wins, `self` fills the gaps.
        let base = ProcessingSettings {
            ocr_languages: Some(vec!["deu".into(), "eng".into()]),
            ocr_min_word_confidence: Some(50),
            ..Default::default()
        };
        let over = ProcessingSettings {
            ocr_languages: Some(vec!["fra".into()]),
            ..Default::default()
        };

        let merged = base.merge(&over);
        assert_eq!(merged.ocr_languages, Some(vec!["fra".to_string()]));
        assert_eq!(
            merged.ocr_min_word_confidence,
            Some(50),
            "a field the override does not mention must survive from the base"
        );
    }

    #[test]
    fn the_language_list_is_validated_before_it_can_starve_the_box() {
        let ok = ProcessingSettings {
            ocr_languages: Some(vec!["deu".into(), "eng".into(), "chi_sim".into()]),
            ..Default::default()
        };
        assert!(ok.validate_ocr().is_ok());

        // Absent is fine — it means "use the default".
        assert!(ProcessingSettings::default().validate_ocr().is_ok());

        // The ISO 639-1 slip the old OcrOptions doc actively encouraged. It
        // would otherwise reach Tesseract and fail there, per image, in a log.
        let iso = ProcessingSettings {
            ocr_languages: Some(vec!["de".into()]),
            ..Default::default()
        };
        let err = iso.validate_ocr().expect_err("'de' names no traineddata");
        assert!(
            err.contains("ISO 639-1"),
            "the error must name the mistake: {err}"
        );

        // The memory guard: 112 languages measured 1.18 GB, and asset jobs run
        // concurrently.
        let many = ProcessingSettings {
            ocr_languages: Some((0..20).map(|i| format!("la{i:02}")).collect()),
            ..Default::default()
        };
        assert!(many.validate_ocr().is_err());

        // A confidence on its own must still be checked — the language
        // early-return used to skip it, which let 200 through as a threshold no
        // word can ever meet, silently discarding every result.
        let only_conf = ProcessingSettings {
            ocr_min_word_confidence: Some(200),
            ..Default::default()
        };
        assert!(
            only_conf.validate_ocr().is_err(),
            "a rule with no languages but a nonsense threshold must not validate"
        );
    }

    #[test]
    fn a_node_with_no_bytes_gets_no_binary_tasks() {
        assert!(ProcessingSettings::default()
            .effective_tasks(None)
            .is_empty());
    }

    #[test]
    fn image_defaults_now_reach_a_real_mime_type() {
        // The regression this whole pass exists for: `ProcessingSettings::image`
        // was unreachable because the rule that selected it matched the literal
        // "image/". Assert the constructor's tasks, and that the family pattern
        // used by `default_rules` selects it for a real upload.
        let s = ProcessingSettings::image();
        assert_eq!(
            s.effective_tasks(Some("image/png")),
            vec!["extract_text", "image_embedding"]
        );
        assert!(crate::rules::mime_matches("image/*", "image/png"));
    }

    #[test]
    fn merge_carries_the_task_list() {
        let base = ProcessingSettings {
            tasks: Some(vec!["extract_text".into()]),
            ..Default::default()
        };
        assert_eq!(
            base.merge(&ProcessingSettings::default()).tasks,
            Some(vec!["extract_text".to_string()])
        );
        let over = ProcessingSettings {
            tasks: Some(vec!["image_ocr".into()]),
            ..Default::default()
        };
        assert_eq!(base.merge(&over).tasks, Some(vec!["image_ocr".to_string()]));
    }

    /// A rule may still NAME `image_caption`; it must be told where the work
    /// happens rather than being told yes.
    ///
    /// Before, the slug was `TaskProvider::Native` with zero call sites, so
    /// `plan_tasks` reported it runnable and the handler answered with one warn
    /// line. `HandledAbove` is the difference between "configured and silently
    /// ignored" and "configured, and here is what performs it".
    #[test]
    fn a_captioning_task_is_reported_handled_above_not_runnable() {
        use crate::rules::tasks::{plan_tasks, BlockedReason};

        let plan = plan_tasks(
            &["image_caption".to_string(), "image_keywords".to_string()],
            |_| false,
        );
        assert!(
            plan.runnable.is_empty(),
            "neither may claim to run here: {:?}",
            plan.runnable
        );
        assert_eq!(plan.blocked.len(), 2);
        for blocked in &plan.blocked {
            match &blocked.blocked {
                BlockedReason::HandledAbove { how } => assert!(
                    how.contains("raisin.ai.completion"),
                    "the reason must name what DOES perform it: {how}"
                ),
                other => panic!("`{}` must be HandledAbove, got {other:?}", blocked.slug),
            }
        }
    }

    /// A settings blob written by the old console — with all six captioning
    /// keys — must still load. Serde ignores unknown fields; this pins that,
    /// because a rule set that fails to deserialize takes the whole routing
    /// table down and every upload with it.
    #[test]
    fn a_rule_written_by_the_old_console_still_deserializes() {
        let json = r#"{
            "pdf_strategy": "auto",
            "generate_image_caption": true,
            "caption_model": "vikhyatk/moondream2",
            "alt_text_prompt": "describe briefly",
            "description_prompt": "describe fully",
            "generate_keywords": true,
            "keywords_prompt": "list keywords",
            "store_extracted_text": true
        }"#;
        let s: ProcessingSettings =
            serde_json::from_str(json).expect("the dead keys must be ignored, not fatal");
        assert_eq!(s.pdf_strategy, Some(PdfStrategy::Auto));
        assert_eq!(s.store_extracted_text, Some(true));
        assert_eq!(
            s.effective_tasks(Some("image/png")),
            vec!["extract_text", "image_embedding"],
            "OCR and the embedding, but the dead captioning keys must not \
             resurrect a task nothing runs"
        );
    }

    #[test]
    fn a_rule_set_persisted_before_tasks_existed_still_deserializes() {
        let json = r#"{"pdf_strategy":"auto","store_extracted_text":true}"#;
        let s: ProcessingSettings = serde_json::from_str(json).unwrap();
        assert!(s.tasks.is_none());
        assert_eq!(
            s.effective_tasks(Some("application/pdf")),
            vec!["extract_text"]
        );
    }
}
