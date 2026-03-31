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

//! Reference extraction utilities
//!
//! Provides functions to extract all PropertyValue::Reference instances from
//! a property tree, tracking their exact paths for indexing and resolution.

use super::value::{PropertyValue, RaisinReference};
use std::collections::HashMap;

/// Extract all Reference values from properties with their dot-notation paths
///
/// Returns a vector of (property_path, RaisinReference) tuples where property_path
/// uses dot notation for objects and numeric indices for arrays.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use raisin_models::nodes::properties::{PropertyValue, RaisinReference, extract_references};
///
/// let mut props = HashMap::new();
/// props.insert("hero".to_string(), PropertyValue::Reference(RaisinReference {
///     id: "123".to_string(),
///     workspace: "ws1".to_string(),
///     path: "/assets/hero.png".to_string(),
/// }));
///
/// let refs = extract_references(&props);
/// assert_eq!(refs.len(), 1);
/// assert_eq!(refs[0].0, "hero");
/// ```
pub fn extract_references(
    properties: &HashMap<String, PropertyValue>,
) -> Vec<(String, RaisinReference)> {
    let mut references = Vec::new();

    for (key, value) in properties {
        extract_references_recursive(key, value, &mut references);
    }

    references
}

/// Recursively extract references from a property value
fn extract_references_recursive(
    current_path: &str,
    value: &PropertyValue,
    references: &mut Vec<(String, RaisinReference)>,
) {
    match value {
        PropertyValue::Reference(ref r) => {
            references.push((current_path.to_string(), r.clone()));
        }
        PropertyValue::Object(obj) => {
            for (k, v) in obj {
                let path = format!("{}.{}", current_path, k);
                extract_references_recursive(&path, v, references);
            }
        }
        PropertyValue::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                let path = format!("{}.{}", current_path, i);
                extract_references_recursive(&path, v, references);
            }
        }
        PropertyValue::Composite(composite) => {
            for (i, element) in composite.items.iter().enumerate() {
                for (k, v) in &element.content {
                    let path = format!("{}.items.{}.{}", current_path, i, k);
                    extract_references_recursive(&path, v, references);
                }
            }
        }
        PropertyValue::Element(element) => {
            for (k, v) in &element.content {
                let path = format!("{}.{}", current_path, k);
                extract_references_recursive(&path, v, references);
            }
        }
        PropertyValue::Resource(resource) => {
            // Resources may contain references in metadata
            if let Some(metadata) = &resource.metadata {
                for (k, v) in metadata {
                    let path = format!("{}.metadata.{}", current_path, k);
                    extract_references_recursive(&path, v, references);
                }
            }
        }
        PropertyValue::Geometry(_) => {
            // Geometry values don't contain references
        }
        _ => {
            // String, Number, Boolean, Date, URL, Vector, Null - no references
        }
    }
}

/// Generate a unique key for a reference target
///
/// Format: `{workspace}:{path}`
pub fn reference_target_key(reference: &RaisinReference) -> String {
    format!("{}:{}", reference.workspace, reference.path)
}

/// Group references by their target, tracking all property paths that reference each target
///
/// Useful for resolving references efficiently - resolve each unique target once,
/// then apply to all property paths that reference it.
///
/// Returns: HashMap<target_key, (property_paths, RaisinReference)>
pub fn group_references_by_target(
    references: Vec<(String, RaisinReference)>,
) -> HashMap<String, (Vec<String>, RaisinReference)> {
    let mut grouped: HashMap<String, (Vec<String>, RaisinReference)> = HashMap::new();

    for (path, reference) in references {
        let key = reference_target_key(&reference);

        grouped
            .entry(key)
            .and_modify(|(paths, _)| paths.push(path.clone()))
            .or_insert_with(|| (vec![path], reference));
    }

    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_reference(id: &str, workspace: &str, path: &str) -> RaisinReference {
        RaisinReference {
            id: id.to_string(),
            workspace: workspace.to_string(),
            path: path.to_string(),
        }
    }

    #[test]
    fn test_extract_simple_reference() {
        let mut props = HashMap::new();
        props.insert(
            "hero".to_string(),
            PropertyValue::Reference(make_reference("123", "ws1", "/assets/hero.png")),
        );

        let refs = extract_references(&props);

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].0, "hero");
        assert_eq!(refs[0].1.path, "/assets/hero.png");
    }

    #[test]
    fn test_extract_nested_object_reference() {
        let mut inner = HashMap::new();
        inner.insert(
            "asset".to_string(),
            PropertyValue::Reference(make_reference("456", "ws1", "/assets/logo.png")),
        );

        let mut props = HashMap::new();
        props.insert("hero".to_string(), PropertyValue::Object(inner));

        let refs = extract_references(&props);

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].0, "hero.asset");
        assert_eq!(refs[0].1.path, "/assets/logo.png");
    }

    #[test]
    fn test_extract_array_reference() {
        let mut props = HashMap::new();
        props.insert(
            "images".to_string(),
            PropertyValue::Array(vec![
                PropertyValue::Reference(make_reference("1", "ws1", "/img1.png")),
                PropertyValue::Reference(make_reference("2", "ws1", "/img2.png")),
            ]),
        );

        let refs = extract_references(&props);

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].0, "images.0");
        assert_eq!(refs[0].1.path, "/img1.png");
        assert_eq!(refs[1].0, "images.1");
        assert_eq!(refs[1].1.path, "/img2.png");
    }

    #[test]
    fn test_extract_complex_nested() {
        let mut inner_obj = HashMap::new();
        inner_obj.insert(
            "background".to_string(),
            PropertyValue::Reference(make_reference("bg1", "ws1", "/bg.png")),
        );

        let mut props = HashMap::new();
        props.insert(
            "sections".to_string(),
            PropertyValue::Array(vec![
                PropertyValue::Object(inner_obj),
                PropertyValue::Reference(make_reference("sec2", "ws1", "/section2.png")),
            ]),
        );

        let refs = extract_references(&props);

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].0, "sections.0.background");
        assert_eq!(refs[0].1.path, "/bg.png");
        assert_eq!(refs[1].0, "sections.1");
        assert_eq!(refs[1].1.path, "/section2.png");
    }

    #[test]
    fn test_reference_target_key() {
        let reference = make_reference("123", "workspace1", "/assets/image.png");
        let key = reference_target_key(&reference);

        assert_eq!(key, "workspace1:/assets/image.png");
    }

    #[test]
    fn test_group_references_by_target() {
        let refs = vec![
            (
                "hero.bg".to_string(),
                make_reference("1", "ws1", "/shared.png"),
            ),
            (
                "footer.logo".to_string(),
                make_reference("2", "ws1", "/shared.png"),
            ),
            (
                "sidebar.icon".to_string(),
                make_reference("3", "ws1", "/other.png"),
            ),
        ];

        let grouped = group_references_by_target(refs);

        assert_eq!(grouped.len(), 2);

        let shared_key = "ws1:/shared.png";
        assert!(grouped.contains_key(shared_key));
        let (paths, _) = &grouped[shared_key];
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"hero.bg".to_string()));
        assert!(paths.contains(&"footer.logo".to_string()));

        let other_key = "ws1:/other.png";
        assert!(grouped.contains_key(other_key));
        let (paths, _) = &grouped[other_key];
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], "sidebar.icon");
    }

    #[test]
    fn test_extract_no_references() {
        let mut props = HashMap::new();
        props.insert(
            "title".to_string(),
            PropertyValue::String("Hello".to_string()),
        );
        props.insert("count".to_string(), PropertyValue::Integer(42));

        let refs = extract_references(&props);

        assert_eq!(refs.len(), 0);
    }
}
