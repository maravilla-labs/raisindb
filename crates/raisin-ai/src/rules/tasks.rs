//! The task vocabulary a processing rule routes a mimetype TO.
//!
//! # Why this exists
//!
//! A processing rule already answers "which nodes" — [`super::RuleMatcher`],
//! now able to express a mimetype family. What it could not answer was "and
//! then WHAT", because the answer was a fixed set of booleans on
//! [`super::ProcessingSettings`]: `extract_pdf_text`, `generate_image_embedding`,
//! `generate_image_caption`, `generate_keywords`. Four flags, hard-coded, so an
//! installation that wanted "docx -> PDF -> page previews -> thumbnail, and
//! markdown -> chunk -> embed" had no way to say it. The pipeline was
//! configuration-shaped on the matching side and hardcoded on the acting side.
//!
//! This module is the acting side's vocabulary. A task is a SLUG, and the set
//! is OPEN: the shape is validated, never the membership. That follows the
//! workflow runtime's `task_type` precedent for the same reason — a closed enum
//! for this lived in four places there before it was opened, and every new
//! member had to be added to all four or it became a deserialization error that
//! took the whole definition down.
//!
//! # Who can actually RUN a task
//!
//! Deliberately not decided here. The set of tasks this process can perform is
//! not a property of the vocabulary; it depends on which plugins the host
//! loaded, and the crate that knows that (`raisin-functions`, via
//! `plugin_method_available`) sits ABOVE this one — `raisin-functions` depends
//! on `raisin-rocksdb` depends on `raisin-ai`, and Cargo rejects package cycles
//! regardless of features. So [`plan_tasks`] takes availability as a predicate
//! and every caller injects what it knows:
//!
//! * the `AssetProcessing` job (in `raisin-rocksdb`) can answer only for native
//!   tasks — it has no way to reach a plugin and must not pretend otherwise;
//! * a function/flow (in or above `raisin-functions`) answers with
//!   `plugin_method_available`.
//!
//! Both get the same vocabulary and the same planner, which is the point.

use serde::Serialize;

/// What has to be present for a task to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskProvider {
    /// Performed by this binary with no plugin — the code is compiled in.
    Native,
    /// Named by a rule, but performed ABOVE this layer — by a trigger
    /// function or flow calling `raisin.ai.completion`, not by any code in
    /// this process.
    ///
    /// # Why this variant exists rather than deleting the slugs
    ///
    /// `image_caption` and `image_keywords` were declared [`Self::Native`] —
    /// "the code is compiled in" — and had ZERO call sites. `plan_tasks`
    /// therefore reported them `runnable`, the job carried booleans saying so,
    /// and the handler answered with one `tracing::warn!` and did nothing. A
    /// rule that asked for captions got a green plan and no captions.
    ///
    /// They are not deleted outright because a rule that names one is not a
    /// typo — it is a correct statement of intent aimed at the wrong layer, and
    /// [`BlockedReason::Unknown`] would tell its author to check their
    /// spelling. This variant lets the plan say what is actually true: the task
    /// is real, it does not happen here, and here is where it does.
    ///
    /// The boundary is deliberate. Extraction is deterministic — same bytes,
    /// same text, no model picker and no prompt — so it belongs in the engine.
    /// A caption is a judgement call with a model and a prompt behind it, and
    /// both of those are product decisions, so it belongs with the product.
    Function {
        /// One line naming what should do it instead.
        how: &'static str,
    },
    /// Performed by a function-binding plugin method (`"<ns>.<method>"`).
    ///
    /// The string is the exact key `plugin_method_available` is asked about, so
    /// a typo here is a task that is permanently "blocked" rather than one that
    /// fails at call time. It is checked by a test against the method names the
    /// media plugin's README documents.
    Plugin {
        /// The plugin method name, e.g. `"media.doc.toPdf"`.
        method: &'static str,
    },
}

/// One task the routing table may name.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct TaskSpec {
    /// The slug written in a rule's `tasks` array.
    pub slug: &'static str,
    /// One line an operator reads in the console.
    pub summary: &'static str,
    /// What has to be present for it to run.
    pub provider: TaskProvider,
}

/// Every task slug this build knows about, native and delegated alike.
///
/// A slug NOT in this table is not an error — see [`plan_tasks`]. The table is
/// documentation and capability mapping, not a whitelist: a plugin shipped
/// after this binary can service a slug nobody here has heard of, and refusing
/// it would make the plugin boundary useless.
pub const KNOWN_TASKS: &[TaskSpec] = &[
    // ---- Native: compiled into this binary ---------------------------------
    TaskSpec {
        slug: "extract_text",
        summary: "Pull text out of the binary into the node's extracted_text property",
        provider: TaskProvider::Native,
    },
    TaskSpec {
        slug: "image_embedding",
        summary: "CLIP embedding of the image itself",
        provider: TaskProvider::Native,
    },
    // ---- Owned by the product layer, NOT by this process --------------------
    //
    // Both write into `raisin:Asset`'s `description` / `alt_text` / `keywords`,
    // which are Vector-indexed — so whatever a trigger writes there becomes a
    // semantic vector through the ordinary write path. Core's half of the
    // contract is the indexing; the model and the prompt are the product's.
    TaskSpec {
        slug: "image_caption",
        summary: "Generate alt text / description from the image",
        provider: TaskProvider::Function {
            how: "a trigger function on asset update calling raisin.ai.completion \
                  with an image content part, writing description / alt_text",
        },
    },
    TaskSpec {
        slug: "image_keywords",
        summary: "Generate keywords from the image",
        provider: TaskProvider::Function {
            how: "a trigger function on asset update calling raisin.ai.completion \
                  with an image content part, writing keywords",
        },
    },
    // ---- Delegated: need the media plugin ----------------------------------
    //
    // These are the transforms the maravilla-media-plugin exposes. They are
    // listed here so a rule can NAME them and the console can say "configured,
    // but this server has no plugin that provides it" — which is a different
    // and far more useful state than the silence an unmatched mimetype used to
    // produce.
    TaskSpec {
        slug: "doc_to_pdf",
        summary: "LibreOffice: office document -> PDF",
        provider: TaskProvider::Plugin {
            method: "media.doc.toPdf",
        },
    },
    TaskSpec {
        slug: "doc_to_markdown",
        summary: "LibreOffice: office document -> Markdown (the text an embedding is made from)",
        provider: TaskProvider::Plugin {
            method: "media.doc.toMarkdown",
        },
    },
    TaskSpec {
        slug: "doc_thumbnail",
        summary: "Render one page of a document to an image (one page per job)",
        provider: TaskProvider::Plugin {
            method: "media.doc.thumbnail",
        },
    },
    TaskSpec {
        slug: "image_resize",
        summary: "Produce a resized derivative / thumbnail",
        provider: TaskProvider::Plugin {
            method: "media.image.resize",
        },
    },
    TaskSpec {
        slug: "image_ocr",
        summary: "OCR the image to text",
        provider: TaskProvider::Plugin {
            method: "media.image.ocr",
        },
    },
    TaskSpec {
        slug: "video_thumbnail",
        summary: "Poster frame from a video",
        provider: TaskProvider::Plugin {
            method: "media.video.thumbnail",
        },
    },
    TaskSpec {
        slug: "video_extract_audio",
        summary: "Split the audio track out of a video",
        provider: TaskProvider::Plugin {
            method: "media.video.extractAudio",
        },
    },
];

/// Look a slug up in [`KNOWN_TASKS`].
pub fn lookup_task(slug: &str) -> Option<&'static TaskSpec> {
    KNOWN_TASKS.iter().find(|t| t.slug == slug)
}

/// Is `slug` a well-formed task name?
///
/// SHAPE, not membership — `[a-z][a-z0-9_]{0,63}`. An unknown-but-well-formed
/// slug is offered to a plugin that may service it; a malformed one is a typo
/// in the rule and is reported as such.
pub fn is_valid_task_slug(slug: &str) -> bool {
    let mut chars = slug.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    slug.len() <= 64 && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Why a named task will not run here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum BlockedReason {
    /// The slug is not `[a-z][a-z0-9_]{0,63}` — almost certainly a typo.
    MalformedSlug,
    /// Known task, and real, but nothing in THIS process performs it — it is
    /// the product layer's job. Not an error and not a typo: the rule is
    /// legible, it is simply aimed a layer down from where the work happens.
    HandledAbove {
        /// What should do it instead.
        how: String,
    },
    /// Known task, but the plugin method that provides it is not loaded.
    PluginMissing {
        /// The method that would have to be available.
        method: String,
    },
    /// Well-formed but not in [`KNOWN_TASKS`] and no plugin claims it.
    Unknown,
}

/// A task that will run, and what will run it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlannedTask {
    /// The slug as written in the rule.
    pub slug: String,
    /// `None` for a native task; the plugin method otherwise.
    pub via: Option<String>,
}

/// A task that will not run, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BlockedTask {
    /// The slug as written in the rule.
    pub slug: String,
    /// Why it cannot run here.
    pub blocked: BlockedReason,
}

/// What a matched rule's task list will actually do on this server.
///
/// Serialised as-is by a management surface, so an operator can ask "what
/// happens when I upload a .docx here" and get an answer instead of an upload
/// that quietly does nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PipelinePlan {
    /// Tasks that will run, in the order the rule listed them.
    pub runnable: Vec<PlannedTask>,
    /// Tasks that were named but cannot run here.
    pub blocked: Vec<BlockedTask>,
}

impl PipelinePlan {
    /// True when at least one task will run.
    pub fn has_work(&self) -> bool {
        !self.runnable.is_empty()
    }

    /// True when a specific slug will run.
    pub fn will_run(&self, slug: &str) -> bool {
        self.runnable.iter().any(|t| t.slug == slug)
    }
}

/// Resolve a rule's task list against what this process can actually do.
///
/// `is_available` is asked about PLUGIN METHOD names (`"media.doc.toPdf"`), not
/// about task slugs — see the module docs for why the answer cannot be computed
/// in this crate. A caller with no plugin access passes `|_| false`, which is
/// honest: it produces `blocked: plugin_missing`, not a silent success.
///
/// A well-formed unknown slug is offered to `is_available` under its own name,
/// so a plugin can service a task this binary predates. It is only reported
/// [`BlockedReason::Unknown`] when nothing claims it.
///
/// Duplicates collapse, keeping first occurrence: a rule that lists
/// `extract_text` twice should extract once, not twice.
pub fn plan_tasks(tasks: &[String], is_available: impl Fn(&str) -> bool) -> PipelinePlan {
    let mut plan = PipelinePlan::default();
    let mut seen: Vec<&str> = Vec::new();

    for slug in tasks {
        let slug = slug.trim();
        if seen.contains(&slug) {
            continue;
        }
        seen.push(slug);

        if !is_valid_task_slug(slug) {
            plan.blocked.push(BlockedTask {
                slug: slug.to_string(),
                blocked: BlockedReason::MalformedSlug,
            });
            continue;
        }

        match lookup_task(slug) {
            Some(spec) => match spec.provider {
                TaskProvider::Native => plan.runnable.push(PlannedTask {
                    slug: slug.to_string(),
                    via: None,
                }),
                TaskProvider::Function { how } => plan.blocked.push(BlockedTask {
                    slug: slug.to_string(),
                    blocked: BlockedReason::HandledAbove {
                        how: how.to_string(),
                    },
                }),
                TaskProvider::Plugin { method } => {
                    if is_available(method) {
                        plan.runnable.push(PlannedTask {
                            slug: slug.to_string(),
                            via: Some(method.to_string()),
                        });
                    } else {
                        plan.blocked.push(BlockedTask {
                            slug: slug.to_string(),
                            blocked: BlockedReason::PluginMissing {
                                method: method.to_string(),
                            },
                        });
                    }
                }
            },
            // Not in this build's table. A plugin newer than this binary may
            // still service it under its own name.
            None if is_available(slug) => plan.runnable.push(PlannedTask {
                slug: slug.to_string(),
                via: Some(slug.to_string()),
            }),
            None => plan.blocked.push(BlockedTask {
                slug: slug.to_string(),
                blocked: BlockedReason::Unknown,
            }),
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn native_tasks_run_with_no_plugin_at_all() {
        let plan = plan_tasks(&s(&["extract_text", "image_embedding"]), |_| false);
        assert_eq!(plan.runnable.len(), 2);
        assert!(plan.runnable.iter().all(|t| t.via.is_none()));
        assert!(plan.blocked.is_empty());
    }

    #[test]
    fn plugin_task_is_blocked_by_name_not_silently_dropped() {
        // The whole point: a .docx rule on a server with no media plugin must
        // SAY that doc_to_markdown cannot run. Before, an unroutable mimetype
        // produced one debug line and no record anywhere.
        let plan = plan_tasks(&s(&["doc_to_markdown"]), |_| false);
        assert!(plan.runnable.is_empty());
        assert_eq!(
            plan.blocked,
            vec![BlockedTask {
                slug: "doc_to_markdown".into(),
                blocked: BlockedReason::PluginMissing {
                    method: "media.doc.toMarkdown".into()
                },
            }]
        );
    }

    #[test]
    fn plugin_task_runs_when_the_method_is_loaded() {
        let plan = plan_tasks(&s(&["doc_to_markdown", "doc_thumbnail"]), |m| {
            m == "media.doc.toMarkdown"
        });
        assert_eq!(plan.runnable.len(), 1);
        assert_eq!(
            plan.runnable[0].via.as_deref(),
            Some("media.doc.toMarkdown")
        );
        assert_eq!(plan.blocked.len(), 1);
    }

    #[test]
    fn availability_is_asked_about_the_method_never_the_slug() {
        // A predicate that only knows slugs must NOT accidentally enable a
        // known task; that would make the plan lie about what will run.
        let plan = plan_tasks(&s(&["doc_to_pdf"]), |name| name == "doc_to_pdf");
        assert!(
            plan.runnable.is_empty(),
            "slug must not satisfy a known task"
        );
    }

    #[test]
    fn unknown_but_well_formed_slug_can_be_serviced_by_a_newer_plugin() {
        let plan = plan_tasks(&s(&["audio_transcribe"]), |m| m == "audio_transcribe");
        assert_eq!(plan.runnable[0].via.as_deref(), Some("audio_transcribe"));

        let plan = plan_tasks(&s(&["audio_transcribe"]), |_| false);
        assert_eq!(plan.blocked[0].blocked, BlockedReason::Unknown);
    }

    #[test]
    fn malformed_slugs_are_reported_as_typos() {
        for bad in ["Extract_Text", "9lives", "doc-to-pdf", "", "with space"] {
            let plan = plan_tasks(&s(&[bad]), |_| true);
            assert_eq!(
                plan.blocked[0].blocked,
                BlockedReason::MalformedSlug,
                "expected {bad:?} to be malformed"
            );
        }
    }

    #[test]
    fn duplicates_collapse_keeping_order() {
        let plan = plan_tasks(
            &s(&["image_embedding", "extract_text", "image_embedding"]),
            |_| false,
        );
        assert_eq!(
            plan.runnable
                .iter()
                .map(|t| t.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["image_embedding", "extract_text"]
        );
    }

    #[test]
    fn every_plugin_task_names_a_media_method_the_plugin_documents() {
        // Guards the one thing a typo here would cause: a task that is
        // permanently blocked, with no error, on a server where the plugin IS
        // loaded. These names come from maravilla-media-plugin's README.
        const DOCUMENTED: &[&str] = &[
            "media.browser.screenshot",
            "media.browser.pdf",
            "media.browser.extractHtml",
            "media.video.render",
            "media.video.transcode",
            "media.video.thumbnail",
            "media.video.extractFrames",
            "media.video.extractAudio",
            "media.doc.toPdf",
            "media.doc.convert",
            "media.doc.toMarkdown",
            "media.doc.toHtml",
            "media.doc.thumbnail",
            "media.doc.replaceImages",
            "media.doc.insertQrCode",
            "media.doc.templateMerge",
            "media.image.resize",
            "media.image.detectFaces",
            "media.image.ocr",
        ];
        for spec in KNOWN_TASKS {
            if let TaskProvider::Plugin { method } = spec.provider {
                assert!(
                    DOCUMENTED.contains(&method),
                    "task {:?} names {method:?}, which the media plugin does not expose",
                    spec.slug
                );
            }
        }
    }

    #[test]
    fn slugs_are_unique_and_well_formed() {
        let mut seen = Vec::new();
        for spec in KNOWN_TASKS {
            assert!(is_valid_task_slug(spec.slug), "bad slug {:?}", spec.slug);
            assert!(!seen.contains(&spec.slug), "duplicate slug {:?}", spec.slug);
            seen.push(spec.slug);
        }
    }
}
