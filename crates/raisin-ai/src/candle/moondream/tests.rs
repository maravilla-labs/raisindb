//! Tests for the Moondream captioner module.

use super::utils::{clean_caption, is_moondream_model, parse_keywords, truncate_for_alt_text};
use super::*;

#[test]
fn test_is_moondream_model() {
    assert!(is_moondream_model("vikhyatk/moondream2"));
    assert!(is_moondream_model("santiagomed/candle-moondream"));
    assert!(is_moondream_model("some/moondream-variant"));
    assert!(!is_moondream_model(
        "Salesforce/blip-image-captioning-large"
    ));
    assert!(!is_moondream_model("openai/clip-vit-base-patch32"));
    assert!(!is_moondream_model("microsoft/git-large-coco"));
}

#[test]
fn test_clean_caption() {
    assert_eq!(clean_caption("  a dog  "), "A dog");
    assert_eq!(clean_caption("<|endoftext|> hello"), "Hello");
    assert_eq!(clean_caption("test <|endoftext|>"), "Test");
}

#[test]
fn test_clean_caption_empty() {
    assert_eq!(clean_caption(""), "");
    assert_eq!(clean_caption("   "), "");
}

#[test]
fn test_truncate_for_alt_text() {
    // Short text - no change
    let short = "A dog playing in the park.";
    assert_eq!(truncate_for_alt_text(short), short);

    // Long text - truncated
    let long = "a".repeat(200);
    let truncated = truncate_for_alt_text(&long);
    assert!(truncated.len() <= 128); // 125 + "..."
    assert!(truncated.ends_with("..."));
}

#[test]
fn test_truncate_for_alt_text_exactly_125() {
    let exact = "x".repeat(125);
    let result = truncate_for_alt_text(&exact);
    assert_eq!(result.len(), 125);
    assert!(!result.ends_with("..."));
}

#[test]
fn test_model_constants() {
    // Default is the quantized candle-compatible model
    assert_eq!(DEFAULT_MOONDREAM_MODEL, "santiagomed/candle-moondream");
    assert_eq!(QUANTIZED_MOONDREAM_MODEL, "santiagomed/candle-moondream");
    // Original moondream2 is available but requires tensor name mapping
    assert_eq!(MOONDREAM2_MODEL, "vikhyatk/moondream2");
    assert_eq!(MOONDREAM_IMAGE_SIZE, 378);
    assert!(!ALT_TEXT_PROMPT.is_empty());
    assert!(!DESCRIPTION_PROMPT.is_empty());
}

#[test]
fn test_prompts_are_different() {
    // Ensure alt-text and description prompts are meaningfully different
    assert_ne!(ALT_TEXT_PROMPT, DESCRIPTION_PROMPT);
    assert!(ALT_TEXT_PROMPT.contains("briefly") || ALT_TEXT_PROMPT.contains("one sentence"));
    assert!(DESCRIPTION_PROMPT.contains("detail"));
}

#[test]
fn test_keywords_prompt_exists() {
    assert!(!KEYWORDS_PROMPT.is_empty());
    // Prompt should ask for listing something (subjects, objects, colors, etc.)
    assert!(
        KEYWORDS_PROMPT.contains("List") || KEYWORDS_PROMPT.contains("list"),
        "Keywords prompt should ask for a list"
    );
}

#[test]
fn test_parse_keywords_comma_separated() {
    let response = "dog, cat, animal, pet, outdoor";
    let keywords = parse_keywords(response);
    assert_eq!(keywords, vec!["dog", "cat", "animal", "pet", "outdoor"]);
}

#[test]
fn test_parse_keywords_json_object() {
    let response = r#"{"keywords": ["dog", "cat", "animal", "pet"]}"#;
    let keywords = parse_keywords(response);
    assert_eq!(keywords, vec!["dog", "cat", "animal", "pet"]);
}

#[test]
fn test_parse_keywords_json_object_with_text() {
    // Model might include some text before/after the JSON
    let response = r#"Here is the response: {"keywords": ["sunset", "beach", "ocean"]}"#;
    let keywords = parse_keywords(response);
    assert_eq!(keywords, vec!["sunset", "beach", "ocean"]);
}

#[test]
fn test_parse_keywords_json_array_wrapped() {
    // Model might wrap the object in an array
    let response = r#"[{"keywords": ["blue", "green", "purple", "gradient"]}]"#;
    let keywords = parse_keywords(response);
    assert_eq!(keywords, vec!["blue", "green", "purple", "gradient"]);
}

#[test]
fn test_parse_keywords_deduplication() {
    // Should deduplicate and limit to 10
    let response = r#"{"keywords": ["blue", "green", "blue", "purple", "green", "blue"]}"#;
    let keywords = parse_keywords(response);
    assert_eq!(keywords, vec!["blue", "green", "purple"]);
}

#[test]
fn test_parse_keywords_json_array() {
    let response = r#"["dog", "cat", "animal", "pet"]"#;
    let keywords = parse_keywords(response);
    assert_eq!(keywords, vec!["dog", "cat", "animal", "pet"]);
}

#[test]
fn test_parse_keywords_json_array_with_text() {
    // Model might include some text before/after the JSON
    let response = r#"Here are the keywords: ["sunset", "beach", "ocean"]"#;
    let keywords = parse_keywords(response);
    assert_eq!(keywords, vec!["sunset", "beach", "ocean"]);
}

#[test]
fn test_parse_keywords_no_spaces() {
    let response = "dog,cat,animal";
    let keywords = parse_keywords(response);
    assert_eq!(keywords, vec!["dog", "cat", "animal"]);
}

#[test]
fn test_parse_keywords_numbered_list() {
    let response = "1. dog\n2. cat\n3. animal";
    let keywords = parse_keywords(response);
    assert_eq!(keywords, vec!["dog", "cat", "animal"]);
}

#[test]
fn test_parse_keywords_bullet_list() {
    let response = "- dog\n- cat\n- animal";
    let keywords = parse_keywords(response);
    assert_eq!(keywords, vec!["dog", "cat", "animal"]);
}

#[test]
fn test_parse_keywords_empty() {
    let keywords = parse_keywords("");
    assert!(keywords.is_empty());
}

#[test]
fn test_parse_keywords_with_whitespace() {
    let response = "  dog , cat  ,  animal  ";
    let keywords = parse_keywords(response);
    assert_eq!(keywords, vec!["dog", "cat", "animal"]);
}
