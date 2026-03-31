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

//! Tests for translation helper functions.

#[cfg(test)]
mod tests {
    use crate::nodes::properties::PropertyValue;
    use crate::translations::helpers::*;
    use crate::translations::types::{JsonPointer, LocaleCode, LocaleOverlay};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn test_locale_fallback_chain() {
        let locale = LocaleCode::parse("en-US").unwrap();
        let chain = locale_fallback_chain(&locale);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].as_str(), "en-US");
        assert_eq!(chain[1].as_str(), "en");

        let locale = LocaleCode::parse("fr").unwrap();
        let chain = locale_fallback_chain(&locale);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].as_str(), "fr");
    }

    #[test]
    fn test_find_best_locale() {
        let requested = LocaleCode::parse("en-US").unwrap();
        let mut available = HashSet::new();
        available.insert(LocaleCode::parse("en").unwrap());
        available.insert(LocaleCode::parse("fr").unwrap());

        let matched = find_best_locale(&requested, &available);
        assert_eq!(matched.unwrap().as_str(), "en");

        let requested = LocaleCode::parse("de-DE").unwrap();
        let matched = find_best_locale(&requested, &available);
        assert!(matched.is_none());
    }

    #[test]
    fn test_is_translatable_field() {
        assert!(is_translatable_field(&JsonPointer::new("/title")));
        assert!(is_translatable_field(&JsonPointer::new("/description")));
        assert!(is_translatable_field(&JsonPointer::new("/content/text")));

        assert!(!is_translatable_field(&JsonPointer::new("/id")));
        assert!(!is_translatable_field(&JsonPointer::new("/created_at")));
        assert!(!is_translatable_field(&JsonPointer::new("/version")));
    }

    #[test]
    fn test_merge_overlays() {
        let mut base = HashMap::new();
        base.insert(
            JsonPointer::new("/title"),
            PropertyValue::String("Base".to_string()),
        );

        let mut override_map = HashMap::new();
        override_map.insert(
            JsonPointer::new("/title"),
            PropertyValue::String("Override".to_string()),
        );

        let merged = merge_overlays(&[
            LocaleOverlay::properties(base),
            LocaleOverlay::properties(override_map),
        ]);

        if let LocaleOverlay::Properties { data } = merged.unwrap() {
            assert_eq!(
                data.get(&JsonPointer::new("/title")),
                Some(&PropertyValue::String("Override".to_string()))
            );
        }
    }

    #[test]
    fn test_filter_overlay_by_prefix() {
        let mut data = HashMap::new();
        data.insert(
            JsonPointer::new("/blocks/1/text"),
            PropertyValue::String("Block 1".to_string()),
        );
        data.insert(
            JsonPointer::new("/blocks/2/text"),
            PropertyValue::String("Block 2".to_string()),
        );
        data.insert(
            JsonPointer::new("/title"),
            PropertyValue::String("Title".to_string()),
        );

        let overlay = LocaleOverlay::properties(data);
        let filtered = filter_overlay_by_prefix(&overlay, &JsonPointer::new("/blocks"));

        assert_eq!(filtered.unwrap().len(), 2);
    }

    #[test]
    fn test_translation_completeness() {
        let mut base = HashMap::new();
        base.insert(
            JsonPointer::new("/title"),
            PropertyValue::String("Title".to_string()),
        );
        base.insert(
            JsonPointer::new("/description"),
            PropertyValue::String("Desc".to_string()),
        );
        let base_overlay = LocaleOverlay::properties(base);

        let mut translated = HashMap::new();
        translated.insert(
            JsonPointer::new("/title"),
            PropertyValue::String("Titre".to_string()),
        );
        let translated_overlay = LocaleOverlay::properties(translated);

        let completeness = translation_completeness(&translated_overlay, &base_overlay);
        assert_eq!(completeness, 50.0);
    }
}
