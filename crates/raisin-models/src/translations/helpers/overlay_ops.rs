// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Overlay operations: merging, filtering, field analysis, and translatability checks.

use crate::translations::types::{JsonPointer, LocaleCode, LocaleOverlay};
use std::collections::{HashMap, HashSet};

/// Check if a JsonPointer references a translatable field.
///
/// Not all fields should be translated. This function checks if a field
/// is typically translatable based on common patterns.
///
/// **Non-translatable fields:**
/// - `id`, `uuid`, `key` - Identifiers
/// - `created_at`, `updated_at` - Timestamps
/// - `version`, `revision` - Versioning
/// - `author`, `owner` - Attribution
/// - `status`, `state` - Status flags
///
/// **Translatable fields:**
/// - `title`, `name` - Names
/// - `description`, `summary` - Descriptions
/// - `content`, `text`, `body` - Content
/// - `label`, `caption` - Labels
/// - `alt`, `title` - Alternative text
///
/// # Arguments
///
/// * `pointer` - The JsonPointer to check
///
/// # Returns
///
/// `true` if the field is typically translatable, `false` otherwise.
///
/// # Note
///
/// This is a heuristic. Actual translatability should be determined
/// by your schema or content model.
///
/// # Examples
///
/// ```rust
/// use raisin_models::translations::{JsonPointer, helpers};
///
/// assert!(helpers::is_translatable_field(&JsonPointer::new("/title")));
/// assert!(helpers::is_translatable_field(&JsonPointer::new("/description")));
/// assert!(helpers::is_translatable_field(&JsonPointer::new("/content/text")));
///
/// assert!(!helpers::is_translatable_field(&JsonPointer::new("/id")));
/// assert!(!helpers::is_translatable_field(&JsonPointer::new("/created_at")));
/// assert!(!helpers::is_translatable_field(&JsonPointer::new("/version")));
/// ```
pub fn is_translatable_field(pointer: &JsonPointer) -> bool {
    // Non-translatable field patterns
    const NON_TRANSLATABLE: &[&str] = &[
        "id",
        "uuid",
        "key",
        "created_at",
        "updated_at",
        "created",
        "updated",
        "modified",
        "timestamp",
        "version",
        "revision",
        "author",
        "owner",
        "creator",
        "status",
        "state",
        "type",
        "kind",
        "format",
        "mime_type",
        "size",
        "width",
        "height",
        "duration",
        "count",
        "order",
        "position",
        "index",
        "parent_id",
        "child_ids",
        "ref",
        "href",
        "url",
        "path",
    ];

    // Translatable field patterns
    const TRANSLATABLE: &[&str] = &[
        "title",
        "name",
        "description",
        "summary",
        "content",
        "text",
        "body",
        "label",
        "caption",
        "alt",
        "placeholder",
        "help",
        "hint",
        "message",
        "note",
        "comment",
        "heading",
        "subheading",
        "tagline",
        "slogan",
        "excerpt",
        "abstract",
    ];

    let last_segment = pointer.last_segment().unwrap_or("");

    // Check if last segment matches non-translatable patterns
    if NON_TRANSLATABLE.contains(&last_segment) {
        return false;
    }

    // Check if last segment matches translatable patterns
    if TRANSLATABLE.contains(&last_segment) {
        return true;
    }

    // Default: consider translatable if it's a string-like field
    // This is conservative - better to allow translation than block it
    true
}

/// Merge multiple locale overlays into a single overlay.
///
/// Combines multiple translation overlays, with later overlays taking
/// precedence over earlier ones. This is useful for combining:
/// - Base translations with user customizations
/// - Multiple translation sources
/// - Incremental translation updates
///
/// # Arguments
///
/// * `overlays` - Slice of overlays to merge (in precedence order)
///
/// # Returns
///
/// A single merged overlay, or `None` if all inputs are Hidden.
///
/// # Behavior
///
/// - If any overlay is `Hidden`, returns `Hidden`
/// - Otherwise, merges all `Properties` overlays
/// - Later overlays override earlier ones for the same field
///
/// # Examples
///
/// ```rust
/// use raisin_models::translations::{LocaleOverlay, JsonPointer, helpers};
/// use raisin_models::nodes::properties::PropertyValue;
/// use std::collections::HashMap;
///
/// let mut base = HashMap::new();
/// base.insert(
///     JsonPointer::new("/title"),
///     PropertyValue::String("Base".to_string())
/// );
///
/// let mut override_map = HashMap::new();
/// override_map.insert(
///     JsonPointer::new("/title"),
///     PropertyValue::String("Override".to_string())
/// );
///
/// let merged = helpers::merge_overlays(&[
///     LocaleOverlay::properties(base),
///     LocaleOverlay::properties(override_map),
/// ]);
///
/// // Later overlay takes precedence
/// if let LocaleOverlay::Properties { data } = merged.unwrap() {
///     assert_eq!(
///         data.get(&JsonPointer::new("/title")),
///         Some(&PropertyValue::String("Override".to_string()))
///     );
/// }
/// ```
pub fn merge_overlays(overlays: &[LocaleOverlay]) -> Option<LocaleOverlay> {
    if overlays.is_empty() {
        return None;
    }

    // Check if any overlay is Hidden
    if overlays.iter().any(|o| o.is_hidden()) {
        return Some(LocaleOverlay::hidden());
    }

    // Merge all Properties overlays
    let mut merged = HashMap::new();

    for overlay in overlays {
        if let Some(properties) = overlay.properties_ref() {
            merged.extend(properties.clone());
        }
    }

    if merged.is_empty() {
        None
    } else {
        Some(LocaleOverlay::properties(merged))
    }
}

/// Filter overlay to only include fields matching a prefix.
///
/// Returns a new overlay containing only fields whose JsonPointer
/// starts with the given prefix.
///
/// # Arguments
///
/// * `overlay` - The overlay to filter
/// * `prefix` - JsonPointer prefix to match
///
/// # Returns
///
/// A new overlay with only matching fields, or `None` if no matches.
///
/// # Examples
///
/// ```rust
/// use raisin_models::translations::{LocaleOverlay, JsonPointer, helpers};
/// use raisin_models::nodes::properties::PropertyValue;
/// use std::collections::HashMap;
///
/// let mut data = HashMap::new();
/// data.insert(
///     JsonPointer::new("/blocks/1/text"),
///     PropertyValue::String("Block 1".to_string())
/// );
/// data.insert(
///     JsonPointer::new("/blocks/2/text"),
///     PropertyValue::String("Block 2".to_string())
/// );
/// data.insert(
///     JsonPointer::new("/title"),
///     PropertyValue::String("Title".to_string())
/// );
///
/// let overlay = LocaleOverlay::properties(data);
/// let filtered = helpers::filter_overlay_by_prefix(
///     &overlay,
///     &JsonPointer::new("/blocks")
/// );
///
/// // Only block fields remain
/// assert_eq!(filtered.unwrap().len(), 2);
/// ```
pub fn filter_overlay_by_prefix(
    overlay: &LocaleOverlay,
    prefix: &JsonPointer,
) -> Option<LocaleOverlay> {
    if overlay.is_hidden() {
        return Some(LocaleOverlay::hidden());
    }

    let properties = overlay.properties_ref()?;
    let filtered: HashMap<_, _> = properties
        .iter()
        .filter(|(ptr, _)| ptr.starts_with(prefix))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    if filtered.is_empty() {
        None
    } else {
        Some(LocaleOverlay::properties(filtered))
    }
}

/// Count the total number of translated fields across multiple overlays.
///
/// # Arguments
///
/// * `overlays` - Map of locales to their overlays
///
/// # Returns
///
/// Total count of translated fields (counting duplicates across locales).
pub fn count_translated_fields(overlays: &HashMap<LocaleCode, LocaleOverlay>) -> usize {
    overlays.values().map(|overlay| overlay.len()).sum()
}

/// Get all unique fields that have translations across all locales.
///
/// Returns a set of JsonPointers that are translated in at least one locale.
///
/// # Arguments
///
/// * `overlays` - Map of locales to their overlays
///
/// # Returns
///
/// Set of JsonPointers that have at least one translation.
pub fn get_translated_fields(
    overlays: &HashMap<LocaleCode, LocaleOverlay>,
) -> HashSet<JsonPointer> {
    let mut fields = HashSet::new();

    for overlay in overlays.values() {
        if let Some(properties) = overlay.properties_ref() {
            fields.extend(properties.keys().cloned());
        }
    }

    fields
}

/// Calculate translation completeness percentage for a locale.
///
/// Compares the number of translated fields in the target locale
/// against a reference locale (typically the base language).
///
/// # Arguments
///
/// * `target_overlay` - The locale overlay to check
/// * `reference_overlay` - The reference overlay (base language)
///
/// # Returns
///
/// Percentage (0.0 to 100.0) of fields translated.
///
/// # Examples
///
/// ```rust
/// use raisin_models::translations::{LocaleOverlay, JsonPointer, helpers};
/// use raisin_models::nodes::properties::PropertyValue;
/// use std::collections::HashMap;
///
/// let mut base = HashMap::new();
/// base.insert(
///     JsonPointer::new("/title"),
///     PropertyValue::String("Title".to_string())
/// );
/// base.insert(
///     JsonPointer::new("/description"),
///     PropertyValue::String("Desc".to_string())
/// );
/// let base_overlay = LocaleOverlay::properties(base);
///
/// let mut translated = HashMap::new();
/// translated.insert(
///     JsonPointer::new("/title"),
///     PropertyValue::String("Titre".to_string())
/// );
/// let translated_overlay = LocaleOverlay::properties(translated);
///
/// let completeness = helpers::translation_completeness(
///     &translated_overlay,
///     &base_overlay
/// );
/// assert_eq!(completeness, 50.0); // 1 out of 2 fields
/// ```
pub fn translation_completeness(
    target_overlay: &LocaleOverlay,
    reference_overlay: &LocaleOverlay,
) -> f64 {
    let reference_fields = match reference_overlay.properties_ref() {
        Some(props) => props.keys().collect::<HashSet<_>>(),
        None => return 100.0, // If reference is hidden, consider complete
    };

    if reference_fields.is_empty() {
        return 100.0;
    }

    let target_fields = match target_overlay.properties_ref() {
        Some(props) => props.keys().collect::<HashSet<_>>(),
        None => return 0.0, // If target is hidden, 0% complete
    };

    let translated_count = reference_fields
        .iter()
        .filter(|field| target_fields.contains(*field))
        .count();

    (translated_count as f64 / reference_fields.len() as f64) * 100.0
}
