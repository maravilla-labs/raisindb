//! Utility functions for Moondream model operations.
//!
//! Includes file discovery, caption cleaning, keyword parsing,
//! and other helper functions.

use std::path::{Path, PathBuf};

use super::super::{CandleError, CandleResult};

/// Find a GGUF model file in a directory.
pub(crate) fn find_gguf_file(path: &Path) -> Option<PathBuf> {
    // Look for common GGUF filenames
    for name in &["model-q4_0.gguf", "model-q4k.gguf", "model.gguf"] {
        let gguf_path = path.join(name);
        if gguf_path.exists() {
            return Some(gguf_path);
        }
    }

    // Try any .gguf file in the directory
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".gguf") {
                return Some(entry.path());
            }
        }
    }

    None
}

/// Find a safetensors model file in a directory.
pub(crate) fn find_model_file(path: &Path) -> CandleResult<PathBuf> {
    // Try safetensors first (preferred)
    let safetensors = path.join("model.safetensors");
    if safetensors.exists() {
        return Ok(safetensors);
    }

    // Try model-00001-of-*.safetensors pattern (sharded models)
    for entry in std::fs::read_dir(path)
        .map_err(|e| CandleError::ModelNotDownloaded(format!("Cannot read directory: {}", e)))?
    {
        let entry = entry.map_err(|e| {
            CandleError::ModelNotDownloaded(format!("Directory entry error: {}", e))
        })?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with("model-") && name_str.ends_with(".safetensors") {
            return Ok(entry.path());
        }
    }

    Err(CandleError::ModelNotDownloaded(format!(
        "Model not found. Expected model.safetensors or model-*.safetensors at {:?}",
        path
    )))
}

/// Clean up generated caption text.
pub(crate) fn clean_caption(caption: &str) -> String {
    let cleaned = caption
        .trim()
        // Remove any trailing/leading special tokens
        .trim_start_matches("<|endoftext|>")
        .trim_end_matches("<|endoftext|>")
        .trim();

    // Capitalize first letter
    let mut chars = cleaned.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Truncate text for alt-text (max ~125 chars for WCAG).
pub(crate) fn truncate_for_alt_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= 125 {
        trimmed.to_string()
    } else {
        let truncated: String = trimmed.chars().take(122).collect();
        format!("{}...", truncated.trim_end())
    }
}

/// Check if a model ID refers to a Moondream model.
pub fn is_moondream_model(model_id: &str) -> bool {
    model_id.to_lowercase().contains("moondream")
}

/// JSON object structure for keywords response.
#[derive(serde::Deserialize)]
struct KeywordsResponse {
    keywords: Vec<String>,
}

/// Deduplicate keywords while preserving order, then limit to max count.
/// Also trims overly long entries and filters out sentence fragments.
fn dedupe_and_limit(keywords: Vec<String>, max: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    keywords
        .into_iter()
        .filter(|s| !s.is_empty())
        // Take only the first part if it looks like a sentence (contains period)
        .map(|s| {
            if let Some(idx) = s.find('.') {
                s[..idx].trim().to_string()
            } else {
                s
            }
        })
        // Truncate very long entries
        .map(|s| {
            if s.len() > 50 {
                s.chars()
                    .take(50)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            } else {
                s
            }
        })
        .filter(|s| !s.is_empty() && seen.insert(s.to_lowercase()))
        .take(max)
        .collect()
}

/// Parse a keyword response into a vector of keywords.
///
/// Handles various formats the model might return:
/// - JSON object: {"keywords": ["keyword1", "keyword2"]}
/// - JSON array of objects: [{"keywords": ["keyword1", "keyword2"]}]
/// - JSON array: ["keyword1", "keyword2", "keyword3"]
/// - Comma-separated: "keyword1, keyword2, keyword3"
/// - Numbered lists: "1. keyword1\n2. keyword2"
/// - Bullet points: "- keyword1\n- keyword2"
pub(crate) fn parse_keywords(response: &str) -> Vec<String> {
    let cleaned = response.trim();

    // Try to parse as JSON object with "keywords" key first
    if let Some(json_start) = cleaned.find('{') {
        if let Some(json_end) = cleaned.rfind('}') {
            let json_str = &cleaned[json_start..=json_end];
            if let Ok(resp) = serde_json::from_str::<KeywordsResponse>(json_str) {
                return dedupe_and_limit(resp.keywords, 10);
            }
        }
    }

    // Try to parse as JSON array (might be array of objects or array of strings)
    if let Some(json_start) = cleaned.find('[') {
        if let Some(json_end) = cleaned.rfind(']') {
            let json_str = &cleaned[json_start..=json_end];

            // Try as array of KeywordsResponse objects first
            if let Ok(arr) = serde_json::from_str::<Vec<KeywordsResponse>>(json_str) {
                if let Some(first) = arr.into_iter().next() {
                    return dedupe_and_limit(first.keywords, 10);
                }
            }

            // Try as simple array of strings
            if let Ok(keywords) = serde_json::from_str::<Vec<String>>(json_str) {
                return dedupe_and_limit(keywords, 10);
            }
        }
    }

    // Check if it looks like a numbered or bulleted list
    if cleaned.contains('\n')
        && (cleaned.starts_with("1.") || cleaned.starts_with("- ") || cleaned.starts_with("• "))
    {
        // Parse as list (newline-separated)
        let keywords: Vec<String> = cleaned
            .lines()
            .map(|line| {
                line.trim()
                    // Remove numbered prefixes like "1. ", "2. "
                    .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
                    // Remove bullet prefixes
                    .trim_start_matches('-')
                    .trim_start_matches('•')
                    .trim()
                    .to_string()
            })
            .collect();
        return dedupe_and_limit(keywords, 10);
    }

    // Parse as comma-separated (fallback)
    let keywords: Vec<String> = cleaned.split(',').map(|s| s.trim().to_string()).collect();
    dedupe_and_limit(keywords, 10)
}
