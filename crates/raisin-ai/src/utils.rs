//! Utility functions for AI provider response processing.
//!
//! Ported from pydantic-ai's _utils.py to handle common LLM response quirks
//! like markdown code fences and other formatting issues.

use once_cell::sync::Lazy;
use regex::Regex;

/// Regex pattern to match JSON inside markdown code fences.
/// Matches: ```json\n{...}\n``` or ```\n{...}\n```
/// Uses DOTALL equivalent ([\s\S]) to match across newlines.
///
/// Pattern breakdown:
/// - ```(?:\w+)? - Opening backticks with optional language (json, etc.)
/// - \s*\n?     - Optional whitespace and newline
/// - (\{[\s\S]*\}) - Capture group: JSON object (including nested braces)
static MARKDOWN_FENCES_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"```(?:\w+)?\s*\n?(\{[\s\S]*\})").unwrap());

/// Strip markdown code fences from a response, returning clean JSON.
///
/// Many LLMs wrap JSON responses in markdown code blocks even when instructed not to.
/// This function extracts the JSON content from patterns like:
/// - ```json\n{...}\n```
/// - ```\n{...}\n```
/// - Already clean JSON starting with `{`
///
/// # Examples
/// ```
/// use raisin_ai::utils::strip_markdown_fences;
///
/// // Already clean JSON - returned as-is
/// assert_eq!(strip_markdown_fences(r#"{"key": "value"}"#), r#"{"key": "value"}"#);
///
/// // JSON in markdown block - extracted
/// assert_eq!(
///     strip_markdown_fences("```json\n{\"key\": \"value\"}\n```"),
///     r#"{"key": "value"}"#
/// );
///
/// // Plain text - returned as-is
/// assert_eq!(strip_markdown_fences("Hello world"), "Hello world");
/// ```
pub fn strip_markdown_fences(text: &str) -> String {
    let trimmed = text.trim();

    // Fast path: already clean JSON
    if trimmed.starts_with('{') {
        return trimmed.to_string();
    }

    // Try to extract JSON from markdown code block
    if let Some(captures) = MARKDOWN_FENCES_PATTERN.captures(trimmed) {
        if let Some(json_match) = captures.get(1) {
            return json_match.as_str().trim().to_string();
        }
    }

    // Return original if no JSON found
    text.to_string()
}

/// Check if a response needs JSON cleanup.
/// Returns true if the text appears to contain JSON wrapped in markdown.
pub fn needs_json_cleanup(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.starts_with('{') && trimmed.contains("```") && trimmed.contains('{')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_already_clean_json() {
        assert_eq!(
            strip_markdown_fences(r#"{"key": "value"}"#),
            r#"{"key": "value"}"#
        );
    }

    #[test]
    fn test_json_with_language_specifier() {
        assert_eq!(
            strip_markdown_fences("```json\n{\"key\": \"value\"}\n```"),
            r#"{"key": "value"}"#
        );
    }

    #[test]
    fn test_json_without_language_specifier() {
        assert_eq!(
            strip_markdown_fences("```\n{\"key\": \"value\"}\n```"),
            r#"{"key": "value"}"#
        );
    }

    #[test]
    fn test_json_with_text_before() {
        // Common pattern: model adds explanation before JSON
        assert_eq!(
            strip_markdown_fences("Here is the JSON:\n\n```json\n{\"key\": \"value\"}\n```"),
            r#"{"key": "value"}"#
        );
    }

    #[test]
    fn test_multiline_json() {
        let input = r#"```json
{
  "description": "A test document",
  "alt_text": "Test",
  "keywords": ["test", "document"]
}
```"#;
        let result = strip_markdown_fences(input);
        assert!(result.contains("\"description\""));
        assert!(result.contains("\"keywords\""));
        assert!(result.starts_with('{'));
        assert!(result.ends_with('}'));
    }

    #[test]
    fn test_no_closing_backticks() {
        // Handle truncated responses
        assert_eq!(
            strip_markdown_fences("```json\n{\"key\": \"value\"}"),
            r#"{"key": "value"}"#
        );
    }

    #[test]
    fn test_plain_text_unchanged() {
        assert_eq!(
            strip_markdown_fences("Just some plain text"),
            "Just some plain text"
        );
    }

    #[test]
    fn test_whitespace_handling() {
        assert_eq!(
            strip_markdown_fences("  \n  {\"key\": \"value\"}  \n  "),
            r#"{"key": "value"}"#
        );
    }

    #[test]
    fn test_nested_json() {
        let input = r#"```json
{
  "outer": {
    "inner": {
      "value": 123
    }
  }
}
```"#;
        let result = strip_markdown_fences(input);
        assert!(result.contains("\"outer\""));
        assert!(result.contains("\"inner\""));
    }

    #[test]
    fn test_needs_cleanup() {
        assert!(!needs_json_cleanup(r#"{"key": "value"}"#));
        assert!(needs_json_cleanup("```json\n{\"key\": \"value\"}\n```"));
        assert!(!needs_json_cleanup("plain text"));
    }
}
