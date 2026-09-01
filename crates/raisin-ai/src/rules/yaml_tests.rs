//! The YAML shape a package's `processing-rules/*.yaml` is written in.
//!
//! These assert the SERDE CONTRACT, not the planner. A package author writes
//! this by hand (or has Claude write it), and a field that silently fails to
//! deserialize gives them a rule that installs and does nothing — the exact
//! "configured and inert" failure the task vocabulary was built to end.

use super::{ProcessingRule, ProcessingSettings, RuleMatchContext, RuleMatcher};

#[test]
fn a_hand_written_office_rule_round_trips() {
    let yaml = r#"
id: office-documents
name: Office documents
order: 10
matcher:
  type: mime_type
  mime_type: application/vnd.openxmlformats-officedocument.*
settings:
  tasks:
    - doc_to_markdown
"#;
    let rule: ProcessingRule = serde_yaml::from_str(yaml).expect("office rule must parse");

    assert_eq!(rule.id, "office-documents");
    assert_eq!(rule.order, 10);
    // `enabled` defaults to true: a package author who omits it means "on".
    assert!(rule.enabled);
    assert_eq!(
        rule.settings.tasks.as_deref(),
        Some(["doc_to_markdown".to_string()].as_slice())
    );

    let ctx = RuleMatchContext::new()
        .with_node_type("raisin:Asset")
        .with_mime_type("application/vnd.openxmlformats-officedocument.wordprocessingml.document");
    assert!(rule.matches(&ctx), "the mime family must match a real docx");
}

#[test]
fn a_list_of_rules_in_one_file_parses() {
    // A package usually wants several that only make sense together, and
    // splitting them across files hides the ORDER — which is load-bearing,
    // because matching is first-match-wins.
    let yaml = r#"
- id: pdfs
  name: PDFs
  order: 1
  matcher: { type: mime_type, mime_type: application/pdf }
  settings:
    tasks: [extract_text]
- id: images
  name: Images
  order: 2
  matcher: { type: mime_type, mime_type: image/* }
  settings:
    tasks: [image_embedding]
"#;
    let rules: Vec<ProcessingRule> = serde_yaml::from_str(yaml).expect("rule list must parse");
    assert_eq!(rules.len(), 2);
    assert_eq!(
        rules[0].settings.tasks.as_deref().unwrap()[0],
        "extract_text"
    );
    assert_eq!(
        rules[1].settings.tasks.as_deref().unwrap()[0],
        "image_embedding"
    );
}

#[test]
fn an_explicitly_empty_task_list_survives_the_round_trip() {
    // `tasks: []` and an ABSENT `tasks` are different configurations: empty
    // means "match these and do nothing" (an opt-out ordered ahead of a broad
    // rule), absent means "fall back to the legacy booleans and the mimetype
    // defaults". Serialising empty as absent would turn a deliberate opt-out
    // into full default processing.
    let rule: ProcessingRule = serde_yaml::from_str(
        r#"
id: ignore-scratch
name: Ignore scratch uploads
matcher: { type: path, pattern: /scratch/** }
settings:
  tasks: []
"#,
    )
    .expect("opt-out rule must parse");

    assert_eq!(rule.settings.tasks.as_deref(), Some([].as_slice()));
    assert!(rule
        .settings
        .effective_tasks(Some("application/pdf"))
        .is_empty());

    let json = serde_json::to_string(&rule.settings).expect("must serialize");
    assert!(
        json.contains("\"tasks\":[]"),
        "an empty task list must stay explicit, got {json}"
    );

    let back: ProcessingSettings = serde_json::from_str(&json).expect("must round trip");
    assert_eq!(back.tasks.as_deref(), Some([].as_slice()));
}

#[test]
fn omitting_tasks_keeps_the_legacy_mime_defaults() {
    // A rule written before `tasks` existed must behave exactly as it did.
    let rule: ProcessingRule = serde_yaml::from_str(
        r#"
id: legacy
name: Legacy
matcher: { type: all }
settings:
  pdf_strategy: auto
  store_extracted_text: true
"#,
    )
    .expect("legacy rule must parse");

    assert!(rule.settings.tasks.is_none());
    assert!(rule
        .settings
        .effective_tasks(Some("application/pdf"))
        .contains(&"extract_text".to_string()));
}

#[test]
fn a_combined_matcher_composes_path_and_mime() {
    let rule: ProcessingRule = serde_yaml::from_str(
        r#"
id: contracts
name: Contracts only
matcher:
  type: combined
  matchers:
    - { type: path, pattern: /contracts/** }
    - { type: mime_type, mime_type: application/pdf }
settings:
  tasks: [extract_text]
"#,
    )
    .expect("combined rule must parse");

    let matching = RuleMatchContext::new()
        .with_path("/contracts/2026/acme.pdf")
        .with_mime_type("application/pdf");
    assert!(rule.matches(&matching));

    // AND, not OR — the right pdf in the wrong place must not match.
    let wrong_path = RuleMatchContext::new()
        .with_path("/invoices/acme.pdf")
        .with_mime_type("application/pdf");
    assert!(!rule.matches(&wrong_path));
}

#[test]
fn a_disabled_rule_never_matches() {
    let rule = ProcessingRule::new("off", "Off")
        .with_matcher(RuleMatcher::All)
        .with_settings(ProcessingSettings::pdf());
    let mut disabled = rule.clone();
    disabled.enabled = false;

    let ctx = RuleMatchContext::new().with_mime_type("application/pdf");
    assert!(rule.matches(&ctx));
    assert!(!disabled.matches(&ctx));
}

/// The OOXML mimetypes differ only near the end and each has a legacy sibling,
/// so a subtype-prefix wildcard is how anyone actually writes an Office rule.
/// It used to fall through to an exact comparison and match nothing at all.
#[test]
fn a_subtype_prefix_wildcard_matches_every_ooxml_type() {
    use super::mime_matches;

    let pattern = "application/vnd.openxmlformats-officedocument.*";
    for actual in [
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ] {
        assert!(
            mime_matches(pattern, actual),
            "{pattern} must match {actual}"
        );
    }

    // Still anchored: it is a prefix, not a substring, and not a free-for-all.
    assert!(!mime_matches(pattern, "application/pdf"));
    assert!(!mime_matches(
        pattern,
        "application/vnd.oasis.opendocument.text"
    ));
    assert!(!mime_matches(pattern, "image/png"));
}

#[test]
fn the_family_and_exact_forms_still_behave() {
    use super::mime_matches;

    // The family form must keep meaning "whole subtype", not "prefix".
    assert!(mime_matches("image/*", "image/png"));
    assert!(!mime_matches("image/*", "imagex/png"));
    assert!(mime_matches("*", "anything/at-all"));
    assert!(mime_matches("*/*", "anything/at-all"));

    // An exact pattern stays exact — no accidental prefixing.
    assert!(mime_matches("application/pdf", "application/pdf"));
    assert!(!mime_matches("application/pdf", "application/pdf-extra"));

    // Parameters on the actual value are ignored, wildcard or not.
    assert!(mime_matches("text/*", "text/plain; charset=utf-8"));
}

/// EVERY matcher must survive the two encodings a rule actually travels in:
/// msgpack (how the repository persists the whole set) and JSON (the HTTP API).
///
/// This is a regression test with a specific history. `NodeType` was a NEWTYPE
/// variant, and serde's internally-tagged representation cannot encode one that
/// wraps a string — it needs a map to add the tag to. So a `node_type` rule
/// could not be serialized at all, and because the repository persists the
/// whole `ProcessingRuleSet` as a single blob, one such rule made saving fail
/// for every other rule in that repository, with a storage error that named
/// nothing useful. It also made the matcher unwritable from a package's YAML.
///
/// A `matches!` assertion would not have caught it: the enum was perfectly
/// usable in memory. Only encoding it does.
#[test]
fn every_matcher_variant_round_trips_through_msgpack_and_json() {
    let variants = vec![
        RuleMatcher::All,
        RuleMatcher::NodeType {
            node_type: "raisin:Asset".to_string(),
        },
        RuleMatcher::Path {
            pattern: "/docs/**".to_string(),
        },
        RuleMatcher::MimeType {
            mime_type: "application/vnd.openxmlformats-officedocument.*".to_string(),
        },
        RuleMatcher::Workspace {
            workspace: "assets".to_string(),
        },
        RuleMatcher::Property {
            name: "kind".to_string(),
            value: "contract".to_string(),
        },
        RuleMatcher::Combined {
            matchers: vec![
                RuleMatcher::NodeType {
                    node_type: "raisin:Asset".to_string(),
                },
                RuleMatcher::MimeType {
                    mime_type: "application/pdf".to_string(),
                },
            ],
        },
    ];

    for (i, matcher) in variants.into_iter().enumerate() {
        let label = format!("{matcher:?}");
        let mut set = super::ProcessingRuleSet::new();
        set.add_rule(
            ProcessingRule::new(format!("r{i}"), format!("Rule {i}")).with_matcher(matcher),
        );

        // msgpack: how the repository stores it.
        let bytes = rmp_serde::to_vec_named(&set)
            .unwrap_or_else(|e| panic!("{label} must encode to msgpack: {e}"));
        let back: super::ProcessingRuleSet = rmp_serde::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("{label} must decode from msgpack: {e}"));
        assert_eq!(back.rules.len(), 1, "{label} lost its rule");

        // JSON: the HTTP API and anything a human reads.
        let json = serde_json::to_string(&set)
            .unwrap_or_else(|e| panic!("{label} must encode to JSON: {e}"));
        let back: super::ProcessingRuleSet = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("{label} must decode from JSON: {e}"));
        assert_eq!(back.rules.len(), 1, "{label} lost its rule");

        // YAML: how a package ships it.
        let yaml = serde_yaml::to_string(&set)
            .unwrap_or_else(|e| panic!("{label} must encode to YAML: {e}"));
        let back: super::ProcessingRuleSet = serde_yaml::from_str(&yaml)
            .unwrap_or_else(|e| panic!("{label} must decode from YAML: {e}"));
        assert_eq!(back.rules.len(), 1, "{label} lost its rule");
    }
}

/// The `node_type` matcher spelled the way a package author writes it, and the
/// way the admin console's TypeScript already declared it.
#[test]
fn a_node_type_matcher_parses_from_the_shape_the_console_sends() {
    let rule: ProcessingRule = serde_yaml::from_str(
        r#"
id: assets-only
name: Assets only
matcher:
  type: node_type
  node_type: raisin:Asset
settings:
  tasks: [extract_text]
"#,
    )
    .expect("node_type rule must parse");

    assert!(rule.matches(&RuleMatchContext::new().with_node_type("raisin:Asset")));
    assert!(!rule.matches(&RuleMatchContext::new().with_node_type("studio:Page")));
}
