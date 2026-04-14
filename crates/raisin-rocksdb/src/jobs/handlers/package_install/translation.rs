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

//! Translation file parsing and overlay conversion for package install/export.
//!
//! Handles `.node.{locale}.yaml` and `{name}.{locale}.yaml` files,
//! converting between YAML and [`LocaleOverlay`] representations.

use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode, LocaleOverlay};
use raisin_validation::field_helpers::NON_TRANSLATABLE_KEYS;
use std::collections::HashMap;

/// Try to extract a locale code from a YAML filename.
///
/// Returns `Some(normalized_locale)` for translation files, `None` otherwise.
/// Uses [`LocaleCode::parse`] to disambiguate from asset metadata filenames
/// (e.g., `.node.index.js.yaml` is not a valid locale).
pub(super) fn parse_translation_locale(yaml_filename: &str) -> Option<String> {
    let without_suffix = yaml_filename.strip_suffix(".yaml")?;

    let candidate = if let Some(inner) = without_suffix.strip_prefix(".node.") {
        // .node.{segment}.yaml — inner must be a valid locale
        if inner.is_empty() {
            return None;
        }
        inner
    } else {
        // {name}.{locale}.yaml — locale is after the last dot
        let dot_pos = without_suffix.rfind('.')?;
        let segment = &without_suffix[dot_pos + 1..];
        if segment.is_empty() {
            return None;
        }
        segment
    };

    LocaleCode::parse(candidate).ok().map(|lc| lc.to_string())
}

/// Derive the base node YAML path from a translation file path.
///
/// - `content/ws/home/.node.de.yaml` → `content/ws/home/.node.yaml`
/// - `content/ws/about.de.yaml` → `content/ws/about.yaml`
pub(super) fn derive_base_node_path(translation_path: &str) -> String {
    let path = std::path::Path::new(translation_path);
    let filename = path.file_name().unwrap_or_default().to_string_lossy();

    let base_filename = if filename.starts_with(".node.") {
        ".node.yaml".to_string()
    } else {
        // about.de.yaml → about.yaml
        let stem = filename.strip_suffix(".yaml").unwrap_or(&filename);
        let dot = stem.rfind('.').unwrap_or(stem.len());
        format!("{}.yaml", &stem[..dot])
    };

    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => {
            format!("{}/{}", dir.display(), base_filename)
        }
        _ => base_filename,
    }
}

/// Derive the node name that the base `.node.yaml` or `{name}.yaml` would produce.
///
/// Replicates the logic in [`ContentNodeDef::derive_name`] for path-based derivation
/// so that translations can look up their target node.
pub(super) fn derive_node_name_from_base_path(base_yaml_path: &str) -> String {
    let path = std::path::Path::new(base_yaml_path);
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    if filename == ".node.yaml" {
        path.parent()
            .and_then(|p| p.file_name())
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        filename
            .strip_suffix(".yaml")
            .unwrap_or(&filename)
            .to_string()
    }
}

/// Convert translation YAML (as `serde_json::Value`) into a [`LocaleOverlay`].
///
/// - `{ hidden: true }` → `LocaleOverlay::Hidden`
/// - Top-level keys → `/{key}` pointers
/// - Arrays of objects with `uuid` → section pointers `/{section}/{uuid}/{field}`
/// - Non-translatable keys are skipped
pub(super) fn yaml_to_overlay(value: serde_json::Value) -> Result<LocaleOverlay> {
    let obj = match value {
        serde_json::Value::Object(map) => map,
        _ => {
            return Err(raisin_error::Error::Validation(
                "Translation YAML must be an object".to_string(),
            ))
        }
    };

    if obj.get("hidden") == Some(&serde_json::Value::Bool(true)) {
        return Ok(LocaleOverlay::hidden());
    }

    let mut data: HashMap<JsonPointer, PropertyValue> = HashMap::new();

    for (key, val) in &obj {
        if NON_TRANSLATABLE_KEYS.contains(&key.as_str()) {
            continue;
        }
        match val {
            serde_json::Value::Array(arr) if is_uuid_section(arr) => {
                collect_section_pointers(&format!("/{}", key), arr, &mut data);
            }
            _ => {
                let pv = json_to_property_value(val);
                data.insert(JsonPointer::new(format!("/{}", key)), pv);
            }
        }
    }

    validate_overlay_pointers(&data)?;

    Ok(LocaleOverlay::properties(data))
}

/// Validate overlay pointers for well-formedness.
///
/// Section pointers (`/{section}/{uuid}/{field}`) must have all non-empty segments.
fn validate_overlay_pointers(data: &HashMap<JsonPointer, PropertyValue>) -> Result<()> {
    for pointer in data.keys() {
        let segs = pointer.segments();
        match segs.len() {
            0 => {
                return Err(raisin_error::Error::Validation(
                    "Empty pointer in overlay".to_string(),
                ))
            }
            1 | 2 => {} // /field or /nested/field — always valid
            _ => {
                // 3+ segments — all must be non-empty
                if segs.iter().any(|s| s.is_empty()) {
                    return Err(raisin_error::Error::Validation(format!(
                        "Section pointer {} has empty segments",
                        pointer
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Convert a [`LocaleOverlay`] back to `serde_json::Value` for YAML export.
///
/// Inverse of [`yaml_to_overlay`]: simple pointers become top-level keys,
/// multi-segment pointers are reconstructed into nested UUID-keyed arrays.
pub(in crate::jobs::handlers) fn overlay_to_yaml(overlay: &LocaleOverlay) -> serde_json::Value {
    match overlay {
        LocaleOverlay::Hidden => serde_json::json!({"hidden": true}),
        LocaleOverlay::Properties { data } => {
            let mut root = serde_json::Map::new();

            for (pointer, value) in data {
                let segs = pointer.segments();
                let jv = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
                insert_at_path(&mut root, &segs, jv);
            }

            serde_json::Value::Object(root)
        }
    }
}

/// Recursively insert a value into a JSON tree following segment pairs.
///
/// Segments follow the pattern: `[key]` for top-level fields, or
/// `[section, uuid, field]` / `[section, uuid, nested_section, nested_uuid, field]`
/// for UUID-keyed array navigation at arbitrary depth.
fn insert_at_path(
    root: &mut serde_json::Map<String, serde_json::Value>,
    segs: &[&str],
    value: serde_json::Value,
) {
    match segs {
        [] => {}
        [key] => {
            root.insert(key.to_string(), value);
        }
        [section, uuid, rest @ ..] if !rest.is_empty() => {
            let arr = root
                .entry(section.to_string())
                .or_insert_with(|| serde_json::Value::Array(Vec::new()));

            if let serde_json::Value::Array(items) = arr {
                // Find existing object with this UUID or create one
                let obj = if let Some(existing) = items.iter_mut().find(|item| {
                    item.as_object()
                        .and_then(|o| o.get("uuid"))
                        .and_then(|u| u.as_str())
                        == Some(uuid)
                }) {
                    existing.as_object_mut().unwrap()
                } else {
                    let mut new_obj = serde_json::Map::new();
                    new_obj.insert(
                        "uuid".to_string(),
                        serde_json::Value::String(uuid.to_string()),
                    );
                    items.push(serde_json::Value::Object(new_obj));
                    items.last_mut().unwrap().as_object_mut().unwrap()
                };

                insert_at_path(obj, rest, value);
            }
        }
        other => {
            // Fallback for unexpected patterns
            root.insert(other.join("."), value);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_uuid_section(arr: &[serde_json::Value]) -> bool {
    !arr.is_empty()
        && arr
            .iter()
            .all(|item| matches!(item, serde_json::Value::Object(obj) if obj.contains_key("uuid")))
}

fn collect_section_pointers(
    prefix: &str,
    arr: &[serde_json::Value],
    data: &mut HashMap<JsonPointer, PropertyValue>,
) {
    for item in arr {
        let serde_json::Value::Object(obj) = item else {
            continue;
        };
        let Some(uuid) = obj.get("uuid").and_then(|v| v.as_str()) else {
            continue;
        };
        for (field, val) in obj {
            if NON_TRANSLATABLE_KEYS.contains(&field.as_str()) {
                continue;
            }
            match val {
                serde_json::Value::Array(nested) if is_uuid_section(nested) => {
                    collect_section_pointers(
                        &format!("{}/{}/{}", prefix, uuid, field),
                        nested,
                        data,
                    );
                }
                _ => {
                    let pointer = JsonPointer::new(format!("{}/{}/{}", prefix, uuid, field));
                    data.insert(pointer, json_to_property_value(val));
                }
            }
        }
    }
}

fn json_to_property_value(val: &serde_json::Value) -> PropertyValue {
    serde_json::from_value(val.clone()).unwrap_or_else(|_| PropertyValue::String(val.to_string()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_translation_locale() {
        // .node.{locale}.yaml variants
        assert_eq!(parse_translation_locale(".node.de.yaml"), Some("de".into()));
        assert_eq!(parse_translation_locale(".node.fr.yaml"), Some("fr".into()));
        assert_eq!(
            parse_translation_locale(".node.en-US.yaml"),
            Some("en-US".into())
        );
        assert_eq!(
            parse_translation_locale(".node.pt-BR.yaml"),
            Some("pt-BR".into())
        );

        // {name}.{locale}.yaml variants
        assert_eq!(parse_translation_locale("about.de.yaml"), Some("de".into()));
        assert_eq!(
            parse_translation_locale("home.en-US.yaml"),
            Some("en-US".into())
        );

        // Not translations
        assert_eq!(parse_translation_locale(".node.yaml"), None);
        assert_eq!(parse_translation_locale(".node.index.js.yaml"), None);
        assert_eq!(parse_translation_locale(".node.script.ts.yaml"), None);
        assert_eq!(parse_translation_locale("about.yaml"), None);
        assert_eq!(parse_translation_locale(".node.de.yml"), None);
    }

    #[test]
    fn test_derive_base_node_path() {
        assert_eq!(
            derive_base_node_path("content/ws/home/.node.de.yaml"),
            "content/ws/home/.node.yaml"
        );
        assert_eq!(
            derive_base_node_path("content/ws/about.de.yaml"),
            "content/ws/about.yaml"
        );
    }

    #[test]
    fn test_yaml_to_overlay_hidden() {
        let overlay = yaml_to_overlay(serde_json::json!({"hidden": true})).unwrap();
        assert!(overlay.is_hidden());
    }

    #[test]
    fn test_yaml_to_overlay_simple() {
        let overlay = yaml_to_overlay(serde_json::json!({
            "title": "Willkommen",
            "description": "Eine Beschreibung"
        }))
        .unwrap();

        let data = overlay.properties_ref().unwrap();
        assert_eq!(data.len(), 2);
        assert!(data.contains_key(&JsonPointer::new("/title")));
        assert!(data.contains_key(&JsonPointer::new("/description")));
    }

    #[test]
    fn test_yaml_to_overlay_sections() {
        let overlay = yaml_to_overlay(serde_json::json!({
            "title": "Startseite",
            "content": [
                {"uuid": "hero-1", "headline": "Vision", "sub": "Bauen"},
                {"uuid": "intro-1", "heading": "Warum?"}
            ]
        }))
        .unwrap();

        let data = overlay.properties_ref().unwrap();
        assert_eq!(data.len(), 4); // title + 2 hero fields + 1 intro field
        assert!(data.contains_key(&JsonPointer::new("/content/hero-1/headline")));
        assert!(data.contains_key(&JsonPointer::new("/content/intro-1/heading")));
    }

    #[test]
    fn test_yaml_to_overlay_skips_non_translatable() {
        let overlay = yaml_to_overlay(serde_json::json!({
            "uuid": "skip", "element_type": "skip", "title": "keep"
        }))
        .unwrap();
        assert_eq!(overlay.len(), 1);
    }

    #[test]
    fn test_overlay_roundtrip() {
        let input = serde_json::json!({"title": "Hola", "description": "Desc"});
        let overlay = yaml_to_overlay(input).unwrap();
        let output = overlay_to_yaml(&overlay);
        assert_eq!(output.get("title").and_then(|v| v.as_str()), Some("Hola"));
    }

    #[test]
    fn test_yaml_to_overlay_validates_empty_segments() {
        // Manually build an overlay with an empty-segment pointer to test validation
        let mut data = HashMap::new();
        data.insert(
            JsonPointer::new("/content//headline"),
            PropertyValue::String("Bad".to_string()),
        );
        let result = validate_overlay_pointers(&data);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty segments"), "error was: {}", err);
    }

    #[test]
    fn test_yaml_to_overlay_nested_sections() {
        // Nested UUID arrays should produce 5-segment pointers
        let overlay = yaml_to_overlay(serde_json::json!({
            "content": [
                {
                    "uuid": "skills-1",
                    "heading": "Faehigkeiten",
                    "categories": [
                        {"uuid": "cat-frontend", "name": "Frontend"},
                        {"uuid": "cat-backend", "name": "Backend"}
                    ]
                }
            ]
        }))
        .unwrap();

        let data = overlay.properties_ref().unwrap();
        // heading + 2 nested names = 3 pointers
        assert_eq!(data.len(), 3);
        assert!(data.contains_key(&JsonPointer::new("/content/skills-1/heading")));
        assert!(data.contains_key(&JsonPointer::new(
            "/content/skills-1/categories/cat-frontend/name"
        )));
        assert!(data.contains_key(&JsonPointer::new(
            "/content/skills-1/categories/cat-backend/name"
        )));
        // Entire categories array should NOT be stored as a single value
        assert!(!data.contains_key(&JsonPointer::new("/content/skills-1/categories")));
    }

    #[test]
    fn test_yaml_to_overlay_nested_without_uuid() {
        // Nested arrays WITHOUT UUIDs should still produce whole-value (current behavior)
        let overlay = yaml_to_overlay(serde_json::json!({
            "content": [
                {
                    "uuid": "skills-1",
                    "heading": "Skills",
                    "categories": [
                        {"name": "Frontend"},
                        {"name": "Backend"}
                    ]
                }
            ]
        }))
        .unwrap();

        let data = overlay.properties_ref().unwrap();
        // heading + categories as whole value = 2 pointers
        assert_eq!(data.len(), 2);
        assert!(data.contains_key(&JsonPointer::new("/content/skills-1/heading")));
        assert!(data.contains_key(&JsonPointer::new("/content/skills-1/categories")));
    }

    #[test]
    fn test_overlay_roundtrip_nested() {
        let input = serde_json::json!({
            "content": [
                {
                    "uuid": "grid-1",
                    "heading": "Projekte",
                    "projects": [
                        {"uuid": "proj-1", "title": "Plattform"},
                        {"uuid": "proj-2", "title": "Blog"}
                    ]
                }
            ]
        });
        let overlay = yaml_to_overlay(input).unwrap();
        let output = overlay_to_yaml(&overlay);

        // Verify structure is reconstructed
        let content = output.get("content").unwrap().as_array().unwrap();
        let grid = content
            .iter()
            .find(|e| e.get("uuid").and_then(|u| u.as_str()) == Some("grid-1"))
            .unwrap();
        assert_eq!(
            grid.get("heading").and_then(|v| v.as_str()),
            Some("Projekte")
        );

        let projects = grid.get("projects").unwrap().as_array().unwrap();
        let proj1 = projects
            .iter()
            .find(|p| p.get("uuid").and_then(|u| u.as_str()) == Some("proj-1"))
            .unwrap();
        assert_eq!(
            proj1.get("title").and_then(|v| v.as_str()),
            Some("Plattform")
        );
    }
}
