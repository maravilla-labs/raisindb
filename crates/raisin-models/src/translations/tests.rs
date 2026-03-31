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

//! Tests for the translation system.
//!
//! This module contains comprehensive tests for all translation types
//! and functionality.

use super::*;
use crate::nodes::properties::PropertyValue;
use raisin_error::Result;
use raisin_hlc::HLC;
use std::collections::HashMap;

// ============================================================================
// LocaleOverlay Tests
// ============================================================================

#[test]
fn test_locale_overlay_properties() {
    let mut data = HashMap::new();
    data.insert(
        JsonPointer::new("/title"),
        PropertyValue::String("Hello".to_string()),
    );

    let overlay = LocaleOverlay::properties(data);
    assert!(!overlay.is_hidden());
    assert_eq!(overlay.len(), 1);
    assert!(!overlay.is_empty());
}

#[test]
fn test_locale_overlay_hidden() {
    let overlay = LocaleOverlay::hidden();
    assert!(overlay.is_hidden());
    assert_eq!(overlay.len(), 0);
    assert!(overlay.is_empty());
}

#[test]
fn test_locale_overlay_properties_ref() {
    let mut data = HashMap::new();
    data.insert(
        JsonPointer::new("/title"),
        PropertyValue::String("Title".to_string()),
    );
    let overlay = LocaleOverlay::properties(data);

    let props = overlay.properties_ref();
    assert!(props.is_some());
    assert_eq!(props.unwrap().len(), 1);

    let hidden = LocaleOverlay::hidden();
    assert!(hidden.properties_ref().is_none());
}

#[test]
fn test_locale_overlay_properties_mut() {
    let mut overlay = LocaleOverlay::properties(HashMap::new());

    if let Some(props) = overlay.properties_mut() {
        props.insert(
            JsonPointer::new("/new_field"),
            PropertyValue::String("Value".to_string()),
        );
    }

    assert_eq!(overlay.len(), 1);
}

// ============================================================================
// JsonPointer Tests
// ============================================================================

#[test]
fn test_json_pointer_new() {
    let ptr = JsonPointer::new("/properties/title");
    assert_eq!(ptr.as_str(), "/properties/title");
}

#[test]
#[should_panic(expected = "JsonPointer must start with '/'")]
fn test_json_pointer_new_invalid() {
    JsonPointer::new("invalid");
}

#[test]
fn test_json_pointer_parse() {
    let ptr = JsonPointer::parse("/valid/path").unwrap();
    assert_eq!(ptr.as_str(), "/valid/path");

    let invalid = JsonPointer::parse("no-slash");
    assert!(invalid.is_err());
}

#[test]
fn test_json_pointer_segments() {
    let ptr = JsonPointer::new("/properties/nested/field");
    assert_eq!(ptr.segments(), vec!["properties", "nested", "field"]);

    let single = JsonPointer::new("/single");
    assert_eq!(single.segments(), vec!["single"]);

    let root = JsonPointer::new("/");
    // Root pointer after "/" has one empty segment
    assert_eq!(root.segments(), vec![""]);
}

#[test]
fn test_json_pointer_parent() {
    let ptr = JsonPointer::new("/properties/nested/field");
    let parent = ptr.parent().unwrap();
    assert_eq!(parent.as_str(), "/properties/nested");

    let parent2 = parent.parent().unwrap();
    assert_eq!(parent2.as_str(), "/properties");

    let root = JsonPointer::new("/single");
    assert!(root.parent().is_none());
}

#[test]
fn test_json_pointer_starts_with() {
    let ptr = JsonPointer::new("/properties/blocks/0/text");
    let prefix = JsonPointer::new("/properties/blocks");
    assert!(ptr.starts_with(&prefix));

    let non_prefix = JsonPointer::new("/other");
    assert!(!ptr.starts_with(&non_prefix));
}

#[test]
fn test_json_pointer_append() {
    let ptr = JsonPointer::new("/properties");
    let child = ptr.append("title");
    assert_eq!(child.as_str(), "/properties/title");

    let nested = child.append("subtitle");
    assert_eq!(nested.as_str(), "/properties/title/subtitle");
}

#[test]
fn test_json_pointer_is_array_index() {
    let array_ptr = JsonPointer::new("/array/0");
    assert!(array_ptr.is_array_index());

    let array_ptr2 = JsonPointer::new("/array/123");
    assert!(array_ptr2.is_array_index());

    let object_ptr = JsonPointer::new("/array/field");
    assert!(!object_ptr.is_array_index());

    let root = JsonPointer::new("/");
    assert!(!root.is_array_index());
}

#[test]
fn test_json_pointer_last_segment() {
    let ptr = JsonPointer::new("/properties/nested/field");
    assert_eq!(ptr.last_segment(), Some("field"));

    let single = JsonPointer::new("/single");
    assert_eq!(single.last_segment(), Some("single"));

    let root = JsonPointer::new("/");
    // Root pointer has an empty string as last segment
    assert_eq!(root.last_segment(), Some(""));
}

#[test]
fn test_json_pointer_display() {
    let ptr = JsonPointer::new("/properties/title");
    assert_eq!(format!("{}", ptr), "/properties/title");
}

#[test]
fn test_json_pointer_from_string() {
    let ptr: JsonPointer = "/path".to_string().into();
    assert_eq!(ptr.as_str(), "/path");

    let ptr: JsonPointer = "/path".into();
    assert_eq!(ptr.as_str(), "/path");
}

// ============================================================================
// LocaleCode Tests
// ============================================================================

#[test]
fn test_locale_code_parse_language_only() {
    let locale = LocaleCode::parse("en").unwrap();
    assert_eq!(locale.as_str(), "en");
    assert_eq!(locale.language(), "en");
    assert_eq!(locale.region(), None);
    assert!(locale.is_language_only());
}

#[test]
fn test_locale_code_parse_language_region() {
    let locale = LocaleCode::parse("en-US").unwrap();
    assert_eq!(locale.as_str(), "en-US");
    assert_eq!(locale.language(), "en");
    assert_eq!(locale.region(), Some("US"));
    assert!(!locale.is_language_only());
}

#[test]
fn test_locale_code_parse_language_script() {
    let locale = LocaleCode::parse("zh-Hans").unwrap();
    // Script codes are uppercased like region codes
    assert_eq!(locale.as_str(), "zh-HANS");
    assert_eq!(locale.language(), "zh");
    assert_eq!(locale.region(), Some("HANS"));
}

#[test]
fn test_locale_code_normalization() {
    let locale = LocaleCode::parse("EN-us").unwrap();
    assert_eq!(locale.as_str(), "en-US");

    let locale = LocaleCode::parse("FR-fr").unwrap();
    assert_eq!(locale.as_str(), "fr-FR");
}

#[test]
fn test_locale_code_invalid() {
    // Too short
    assert!(LocaleCode::parse("e").is_err());

    // Too long
    assert!(LocaleCode::parse("engl").is_err());

    // Invalid region
    assert!(LocaleCode::parse("en-U").is_err());

    // Too many parts
    assert!(LocaleCode::parse("en-US-extra").is_err());

    // Empty
    assert!(LocaleCode::parse("").is_err());
}

#[test]
fn test_locale_code_parent() {
    let locale = LocaleCode::parse("en-US").unwrap();
    let parent = locale.parent().unwrap();
    assert_eq!(parent.as_str(), "en");
    assert!(parent.is_language_only());

    assert!(parent.parent().is_none());

    let language_only = LocaleCode::parse("fr").unwrap();
    assert!(language_only.parent().is_none());
}

#[test]
fn test_locale_code_matches() {
    let base = LocaleCode::parse("en").unwrap();
    let specific = LocaleCode::parse("en-US").unwrap();
    let other = LocaleCode::parse("fr").unwrap();

    // Base matches specific (parent relationship)
    assert!(base.matches(&specific));

    // Exact match
    assert!(specific.matches(&specific));
    assert!(base.matches(&base));

    // Specific does not match base (not a parent)
    assert!(!specific.matches(&base));

    // Different languages don't match
    assert!(!base.matches(&other));
    assert!(!specific.matches(&other));
}

#[test]
fn test_locale_code_display() {
    let locale = LocaleCode::parse("en-US").unwrap();
    assert_eq!(format!("{}", locale), "en-US");
}

#[test]
fn test_locale_code_from_into() {
    let locale = LocaleCode::parse("en-US").unwrap();
    let string: String = locale.clone().into();
    assert_eq!(string, "en-US");

    let locale_result: Result<LocaleCode> = "fr-FR".try_into();
    assert!(locale_result.is_ok());

    let locale_result: Result<LocaleCode> = "invalid".try_into();
    assert!(locale_result.is_err());
}

// ============================================================================
// TranslationMeta Tests
// ============================================================================

#[test]
fn test_translation_meta_new() {
    let locale = LocaleCode::parse("fr-FR").unwrap();
    let meta = TranslationMeta::new(
        locale.clone(),
        HLC::new(42, 0),
        Some(HLC::new(41, 0)),
        "translator@example.com".to_string(),
        "Add French translation".to_string(),
    );

    assert_eq!(meta.locale, locale);
    assert_eq!(meta.revision, HLC::new(42, 0));
    assert_eq!(meta.parent_revision, Some(HLC::new(41, 0)));
    assert_eq!(meta.actor, "translator@example.com");
    assert_eq!(meta.message, "Add French translation");
    assert!(!meta.is_system);
    assert!(!meta.is_initial());
}

#[test]
fn test_translation_meta_system() {
    let locale = LocaleCode::parse("en").unwrap();
    let meta = TranslationMeta::system(locale.clone(), HLC::new(1, 0), "System init".to_string());

    assert_eq!(meta.locale, locale);
    assert_eq!(meta.revision, HLC::new(1, 0));
    assert_eq!(meta.actor, "system");
    assert_eq!(meta.message, "System init");
    assert!(meta.is_system);
    assert!(meta.is_initial());
}

#[test]
fn test_translation_meta_with_timestamp() {
    let locale = LocaleCode::parse("ja").unwrap();
    let timestamp = chrono::Utc::now();
    let meta = TranslationMeta::with_timestamp(
        locale.clone(),
        HLC::new(42, 0),
        Some(HLC::new(41, 0)),
        timestamp,
        "user@example.com".to_string(),
        "Import".to_string(),
        false,
    );

    assert_eq!(meta.timestamp, timestamp);
    assert_eq!(meta.locale, locale);
}

#[test]
fn test_translation_meta_is_initial() {
    let locale = LocaleCode::parse("en").unwrap();

    let initial = TranslationMeta::system(locale.clone(), HLC::new(1, 0), "Initial".to_string());
    assert!(initial.is_initial());

    let update = TranslationMeta::new(
        locale,
        HLC::new(2, 0),
        Some(HLC::new(1, 0)),
        "user@example.com".to_string(),
        "Update".to_string(),
    );
    assert!(!update.is_initial());
}

#[test]
fn test_translation_meta_age() {
    let locale = LocaleCode::parse("en").unwrap();
    let meta = TranslationMeta::system(locale, HLC::new(1, 0), "Test".to_string());

    let age = meta.age();
    assert!(age.num_seconds() < 5); // Should be very recent

    assert!(!meta.is_older_than(chrono::Duration::hours(1)));
}

#[test]
fn test_translation_meta_builder() {
    let locale = LocaleCode::parse("en").unwrap();
    let meta = TranslationMetaBuilder::new(
        locale.clone(),
        HLC::new(42, 0),
        "user@example.com".to_string(),
    )
    .message("Test message".to_string())
    .parent_revision(Some(HLC::new(41, 0)))
    .build();

    assert_eq!(meta.locale, locale);
    assert_eq!(meta.revision, HLC::new(42, 0));
    assert_eq!(meta.parent_revision, Some(HLC::new(41, 0)));
    assert_eq!(meta.message, "Test message");
    assert!(!meta.is_system);
}

#[test]
fn test_translation_meta_builder_system() {
    let locale = LocaleCode::parse("en").unwrap();
    let meta = TranslationMetaBuilder::new(locale.clone(), HLC::new(1, 0), "system".to_string())
        .system()
        .message("System change".to_string())
        .build();

    assert_eq!(meta.locale, locale);
    assert!(meta.is_system);
    assert_eq!(meta.message, "System change");
}
