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
    let matcher = RuleMatcher::NodeType("raisin:Asset".to_string());
    let context = RuleMatchContext::new().with_node_type("raisin:Asset");
    assert!(matcher.matches(&context));

    let context2 = RuleMatchContext::new().with_node_type("raisin:Document");
    assert!(!matcher.matches(&context2));
}

#[test]
fn test_rule_matcher_combined() {
    let matcher = RuleMatcher::Combined {
        matchers: vec![
            RuleMatcher::NodeType("raisin:Asset".to_string()),
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
        generate_image_caption: Some(true),
        ..Default::default()
    };

    let merged = base.merge(&override_settings);
    assert_eq!(merged.pdf_strategy, Some(PdfStrategy::OcrOnly));
    assert_eq!(merged.generate_image_embedding, Some(true));
    assert_eq!(merged.generate_image_caption, Some(true));
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
