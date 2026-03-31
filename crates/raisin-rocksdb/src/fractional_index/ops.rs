//! Core fractional index operations.

use raisin_error::Result;

const WARNING_LENGTH: usize = 48;

/// Generate first label for an empty sequence
///
/// This creates an initial fractional index that can be used as the first
/// element in an ordered sequence.
pub fn first() -> String {
    ::fractional_index::FractionalIndex::default().to_string()
}

/// Increment a label to get the next position (append after)
///
/// Creates a new label that sorts lexicographically after the given label.
pub fn inc(label: &str) -> Result<String> {
    let index = ::fractional_index::FractionalIndex::from_string(label)
        .map_err(|e| raisin_error::Error::storage(format!("Invalid fractional index: {}", e)))?;

    let next = ::fractional_index::FractionalIndex::new_after(&index);

    Ok(next.to_string())
}

/// Decrement a label to get the previous position (prepend before)
///
/// Creates a new label that sorts lexicographically before the given label.
pub fn prev(label: &str) -> Result<String> {
    let index = ::fractional_index::FractionalIndex::from_string(label)
        .map_err(|e| raisin_error::Error::storage(format!("Invalid fractional index: {}", e)))?;

    let before = ::fractional_index::FractionalIndex::new_before(&index);

    Ok(before.to_string())
}

/// Calculate midpoint between two labels
///
/// Creates a new label that sorts lexicographically between the two given labels.
///
/// # Errors
///
/// Returns an error if either label is invalid or `a` is not less than `b`.
pub fn mid(a: &str, b: &str) -> Result<String> {
    if a >= b {
        return Err(raisin_error::Error::storage(
            "First label must be less than second label",
        ));
    }

    let index_a = ::fractional_index::FractionalIndex::from_string(a)
        .map_err(|e| raisin_error::Error::storage(format!("Invalid fractional index a: {}", e)))?;
    let index_b = ::fractional_index::FractionalIndex::from_string(b)
        .map_err(|e| raisin_error::Error::storage(format!("Invalid fractional index b: {}", e)))?;

    let middle = ::fractional_index::FractionalIndex::new_between(&index_a, &index_b)
        .ok_or_else(|| raisin_error::Error::storage("Cannot create midpoint between labels"))?;

    Ok(middle.to_string())
}

/// Generate a label between two optional labels
///
/// This is the main entry point for calculating order labels. It handles all cases:
/// - First element: `between(None, None)`
/// - Prepend: `between(None, Some(first))`
/// - Append: `between(Some(last), None)`
/// - Insert: `between(Some(before), Some(after))`
pub fn between(before: Option<&str>, after: Option<&str>) -> Result<String> {
    match (before, after) {
        (None, None) => Ok(first()),
        (None, Some(after_label)) => prev(after_label),
        (Some(before_label), None) => inc(before_label),
        (Some(before_label), Some(after_label)) => mid(before_label, after_label),
    }
}

/// Check if a label is approaching exhaustion
///
/// Returns `true` if the label length exceeds the warning threshold,
/// indicating that the ordering space is becoming fragmented.
pub fn is_approaching_exhaustion(label: &str) -> bool {
    label.len() >= WARNING_LENGTH
}

/// Extract the fractional part from an order label (strips ::HLC suffix)
///
/// Order labels have the format `{fractional}::{HLC_hex}` where:
/// - `fractional` is the actual fractional index used for ordering
/// - `HLC_hex` is a 16-character hex timestamp for causal ordering
///
/// This function extracts just the fractional part by finding the LAST `::`
/// occurrence (since fractional indices may theoretically contain `:`).
pub fn extract_fractional(label: &str) -> &str {
    label.rfind("::").map(|pos| &label[..pos]).unwrap_or(label)
}
