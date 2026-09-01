//! Tests for the rules module.

use super::*;
use crate::pdf::PdfStrategy;

#[test]
fn test_glob_match_basic() {
    use super::matcher::glob_match;
    assert!(glob_match("/docs/file.pdf", "/docs/file.pdf"));
    assert!(!glob_match("/docs/file.pdf", "/docs/other.pdf"));
}

#[test]
fn test_glob_match_star() {
    use super::matcher::glob_match;
    assert!(glob_match("/docs/*.pdf", "/docs/file.pdf"));
    assert!(glob_match("/docs/*.pdf", "/docs/report.pdf"));
    assert!(!glob_match("/docs/*.pdf", "/docs/file.txt"));
    assert!(!glob_match("/docs/*.pdf", "/other/file.pdf"));
}

#[test]
fn test_glob_match_double_star() {
    use super::matcher::glob_match;
    assert!(glob_match("/docs/**", "/docs/file.pdf"));
    assert!(glob_match("/docs/**", "/docs/sub/file.pdf"));
    assert!(glob_match("/docs/**", "/docs/a/b/c/file.pdf"));
    assert!(!glob_match("/docs/**", "/other/file.pdf"));
}

#[test]
fn test_glob_match_double_star_middle() {
    use super::matcher::glob_match;
    assert!(glob_match("/docs/**/file.pdf", "/docs/file.pdf"));
    assert!(glob_match("/docs/**/file.pdf", "/docs/sub/file.pdf"));
    assert!(glob_match("/docs/**/file.pdf", "/docs/a/b/file.pdf"));
    assert!(!glob_match("/docs/**/file.pdf", "/docs/other.pdf"));
}

#[test]
fn test_glob_match_question() {
    use super::matcher::glob_match;
    assert!(glob_match("/docs/file?.pdf", "/docs/file1.pdf"));
    assert!(glob_match("/docs/file?.pdf", "/docs/fileA.pdf"));
    assert!(!glob_match("/docs/file?.pdf", "/docs/file12.pdf"));
}

#[test]
fn test_rule_matcher_node_type() {
    let matcher = RuleMatcher::NodeType {
        node_type: "raisin:Asset".to_string(),
    };
    let context = RuleMatchContext::new().with_node_type("raisin:Asset");
    assert!(matcher.matches(&context));

    let context2 = RuleMatchContext::new().with_node_type("raisin:Document");
    assert!(!matcher.matches(&context2));
}

#[test]
fn test_rule_matcher_combined() {
    let matcher = RuleMatcher::Combined {
        matchers: vec![
            RuleMatcher::NodeType {
                node_type: "raisin:Asset".to_string(),
            },
            RuleMatcher::Path {
                pattern: "/docs/**".to_string(),
            },
        ],
    };

    let context = RuleMatchContext::new()
        .with_node_type("raisin:Asset")
        .with_path("/docs/file.pdf");
    assert!(matcher.matches(&context));

    let context2 = RuleMatchContext::new()
        .with_node_type("raisin:Asset")
        .with_path("/other/file.pdf");
    assert!(!matcher.matches(&context2));
}

#[test]
fn test_rule_set_first_match_wins() {
    let mut set = ProcessingRuleSet::new();

    set.add_rule(
        ProcessingRule::new("specific", "Specific PDFs")
            .with_order(1)
            .with_matcher(RuleMatcher::Path {
                pattern: "/docs/**".to_string(),
            })
            .with_settings(ProcessingSettings {
                pdf_strategy: Some(PdfStrategy::NativeOnly),
                ..Default::default()
            }),
    );

    set.add_rule(
        ProcessingRule::new("default", "Default")
            .with_order(10)
            .with_matcher(RuleMatcher::All)
            .with_settings(ProcessingSettings {
                pdf_strategy: Some(PdfStrategy::Auto),
                ..Default::default()
            }),
    );

    let context = RuleMatchContext::new().with_path("/docs/report.pdf");
    let rule = set.find_matching_rule(&context).unwrap();
    assert_eq!(rule.id, "specific");
    assert_eq!(rule.settings.pdf_strategy, Some(PdfStrategy::NativeOnly));

    let context2 = RuleMatchContext::new().with_path("/other/file.pdf");
    let rule2 = set.find_matching_rule(&context2).unwrap();
    assert_eq!(rule2.id, "default");
}

#[test]
fn test_processing_settings_merge() {
    let base = ProcessingSettings {
        pdf_strategy: Some(PdfStrategy::Auto),
        generate_image_embedding: Some(true),
        ..Default::default()
    };

    let override_settings = ProcessingSettings {
        pdf_strategy: Some(PdfStrategy::OcrOnly),
        trigger_embedding: Some(true),
        ..Default::default()
    };

    let merged = base.merge(&override_settings);
    assert_eq!(merged.pdf_strategy, Some(PdfStrategy::OcrOnly));
    assert_eq!(merged.generate_image_embedding, Some(true));
    assert_eq!(merged.trigger_embedding, Some(true));
}

#[test]
fn test_deserialize_invalid_chunking() {
    let json = r#"{"chunking": true}"#;
    let res: Result<ProcessingSettings, _> = serde_json::from_str(json);
    assert!(
        res.is_ok(),
        "Should handle boolean true as default chunking config"
    );
    let settings = res.unwrap();
    assert!(settings.chunking.is_some());
    assert_eq!(settings.chunking.unwrap().chunk_size, 256);

    let json_false = r#"{"chunking": false}"#;
    let res_false: Result<ProcessingSettings, _> = serde_json::from_str(json_false);
    assert!(res_false.is_ok());
    assert!(res_false.unwrap().chunking.is_none());
}

// =============================================================================
// MIME matching — the routing table's primary key
// =============================================================================

#[test]
fn mime_family_pattern_matches_every_subtype() {
    use crate::rules::mime_matches;
    for actual in ["image/png", "image/jpeg", "image/avif", "image/svg+xml"] {
        assert!(
            mime_matches("image/*", actual),
            "image/* should match {actual}"
        );
    }
    assert!(!mime_matches("image/*", "application/pdf"));
    // A family pattern must NOT behave like a path glob: `*` never crosses the
    // `/`, or "every image" would quietly become "everything".
    assert!(!mime_matches("image/*", "text/plain"));
}

#[test]
fn the_shipped_default_image_rule_now_matches_an_actual_upload() {
    // Regression. `default_rules()` shipped `mime_type: "image/"` against an
    // exact-equality matcher, so the image rule matched nothing that has ever
    // been uploaded — every image silently fell through to the catch-all and
    // `ProcessingSettings::image()` was unreachable code.
    let set = ProcessingRuleSet::default_rules();
    let ctx = RuleMatchContext::new()
        .with_node_type("raisin:Asset")
        .with_mime_type("image/png");
    let rule = set.find_matching_rule(&ctx).expect("a rule must match");
    assert_eq!(
        rule.id, "image-default",
        "an image must reach the image rule"
    );
    assert_eq!(
        rule.settings.effective_tasks(Some("image/png")),
        vec!["extract_text", "image_embedding"],
        "OCR and the CLIP embedding are both shipped defaults for an image — \
         the text in a screenshot or a scanned page is otherwise findable only \
         by filename. `image_caption` is NOT: it is a product-layer task, so \
         listing it here would put a permanently-blocked entry in the plan of \
         every image upload on every server"
    );
}

#[test]
fn the_legacy_trailing_slash_spelling_still_selects_the_family() {
    // Rule sets already persisted in CF_PROCESSING_RULES carry `"image/"`.
    // They must start working, not need a migration.
    use crate::rules::mime_matches;
    assert!(mime_matches("image/", "image/png"));
    assert!(!mime_matches("image/", "video/mp4"));
}

#[test]
fn mime_parameters_on_the_actual_value_are_ignored() {
    use crate::rules::mime_matches;
    assert!(mime_matches("text/plain", "text/plain; charset=utf-8"));
    assert!(mime_matches("text/*", "text/markdown;charset=utf-8"));
}

#[test]
fn mime_matching_is_case_insensitive_and_rejects_empties() {
    use crate::rules::mime_matches;
    assert!(mime_matches("Application/PDF", "application/pdf"));
    assert!(mime_matches("IMAGE/*", "image/png"));
    assert!(!mime_matches("", "image/png"));
    assert!(!mime_matches("image/*", ""));
}

#[test]
fn a_wildcard_mime_rule_requires_bytes_where_all_does_not() {
    // `MimeType { "*" }` is how you say "any asset that has a binary".
    // `RuleMatcher::All` also matches a plain content node with no file.
    use crate::rules::mime_matches;
    assert!(mime_matches("*", "video/mp4"));
    assert!(mime_matches("*/*", "video/mp4"));

    let no_mime = RuleMatchContext::new().with_node_type("studio:Page");
    assert!(RuleMatcher::All.matches(&no_mime));
    assert!(!RuleMatcher::MimeType {
        mime_type: "*".into()
    }
    .matches(&no_mime));
}

#[test]
fn an_office_document_is_asked_for_text_even_where_nothing_can_read_it() {
    // Out of the box a .docx reaches the catch-all rule, and that rule now
    // ASKS for text extraction. Nothing in core can perform it, so the asset
    // job records `__extract_status = 'unsupported'` — a durable, queryable
    // row that a backfill finds the day a media plugin gains the format.
    //
    // The previous baseline was an empty task list, which meant the upload did
    // nothing AND left no trace of having done nothing. Naming
    // `doc_to_markdown` in a rule additionally lets the planner say WHICH
    // plugin method is missing, which the artifact then records as its detail.
    let set = ProcessingRuleSet::default_rules();
    let ctx = RuleMatchContext::new()
        .with_node_type("raisin:Asset")
        .with_mime_type("application/vnd.openxmlformats-officedocument.wordprocessingml.document");
    let rule = set.find_matching_rule(&ctx).unwrap();
    assert_eq!(rule.id, "default");
    assert_eq!(
        rule.settings.effective_tasks(ctx.mime_type.as_deref()),
        vec!["extract_text"]
    );
}

/// The two-tier design in one assertion: the SAME rule, planned twice.
///
/// A stock server has no plugin and must say so by name; a Studio server that
/// loaded the media plugin must run the very same configuration. Before the
/// capability probe existed, the enqueue path planned against a hard-coded
/// `|_| false`, so only the first column was reachable and a `.docx` rule was
/// dead on every server there is.
#[test]
fn one_rule_plans_differently_on_a_stock_and_a_plugin_server() {
    use crate::rules::tasks::{plan_tasks, BlockedReason};

    let tasks = vec!["doc_to_markdown".to_string()];

    let stock = plan_tasks(&tasks, |_| false);
    assert!(stock.runnable.is_empty());
    assert_eq!(
        stock.blocked[0].blocked,
        BlockedReason::PluginMissing {
            method: "media.doc.toMarkdown".to_string()
        }
    );

    let studio = plan_tasks(&tasks, |m| m == "media.doc.toMarkdown");
    assert!(studio.blocked.is_empty());
    assert_eq!(studio.runnable[0].slug, "doc_to_markdown");
    // `via` is what the asset job partitions on to decide it cannot perform
    // this itself. A plugin task that reported `via: None` would be handed to a
    // process with no way to reach a plugin.
    assert_eq!(
        studio.runnable[0].via.as_deref(),
        Some("media.doc.toMarkdown")
    );
}

/// A native task keeps `via: None` even on a server full of plugins, because
/// that is the flag the job splits on.
#[test]
fn a_native_task_is_never_reported_as_delegated() {
    use crate::rules::tasks::plan_tasks;

    let plan = plan_tasks(&["extract_text".to_string()], |_| true);
    assert_eq!(plan.runnable[0].slug, "extract_text");
    assert!(plan.runnable[0].via.is_none());
}

/// Captioning stays aimed at the product layer no matter what is loaded. A
/// plugin cannot claim it by accident: it is `Function`, not `Plugin`, so
/// `is_available` is never consulted for it.
#[test]
fn a_caption_task_is_handled_above_even_with_every_plugin_present() {
    use crate::rules::tasks::{plan_tasks, BlockedReason};

    let plan = plan_tasks(&["image_caption".to_string()], |_| true);
    assert!(plan.runnable.is_empty());
    assert!(matches!(
        plan.blocked[0].blocked,
        BlockedReason::HandledAbove { .. }
    ));
}
