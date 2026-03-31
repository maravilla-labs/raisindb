// TODO(v0.2): Field-level security filtering for RLS
#![allow(dead_code)]

//! Field-level security filtering.
//!
//! Filters node properties based on permission rules:
//! - `fields`: Whitelist - only these fields are visible
//! - `except_fields`: Blacklist - these fields are hidden
//!
//! If both are specified, `fields` takes precedence.

use raisin_models::nodes::Node;
use raisin_models::permissions::Permission;

/// Filter node fields based on permission rules.
///
/// This modifies the node in place, removing properties that the user
/// is not authorized to see.
///
/// # Arguments
///
/// * `node` - The node to filter (modified in place)
/// * `permission` - The permission containing field rules
///
/// # Rules
///
/// - If `fields` is Some: only keep fields in the whitelist
/// - If `except_fields` is Some: remove fields in the blacklist
/// - If both are None: no filtering applied
pub fn filter_node_fields(node: &mut Node, permission: &Permission) {
    // Whitelist takes precedence
    if let Some(allowed_fields) = &permission.fields {
        node.properties
            .retain(|key, _| allowed_fields.contains(key));
        return;
    }

    // Apply blacklist
    if let Some(denied_fields) = &permission.except_fields {
        node.properties
            .retain(|key, _| !denied_fields.contains(key));
    }
}

/// Filter multiple nodes based on their applicable permissions.
///
/// This applies field-level filtering to each node based on the most
/// specific permission that matches.
///
/// # Arguments
///
/// * `nodes` - The nodes to filter (modified in place)
/// * `permissions` - Available permissions to check against
pub fn filter_nodes_fields(nodes: &mut [Node], permissions: &[Permission]) {
    for node in nodes {
        // Find the most specific permission that matches this node's path
        if let Some(permission) = find_matching_permission(&node.path, permissions) {
            filter_node_fields(node, permission);
        }
    }
}

/// Find the most specific permission that matches a path.
///
/// More specific patterns (longer, less wildcards) are preferred.
fn find_matching_permission<'a>(
    path: &str,
    permissions: &'a [Permission],
) -> Option<&'a Permission> {
    use super::path_matcher::matches_path_pattern;

    let mut best_match: Option<(&Permission, usize)> = None;

    for permission in permissions {
        if matches_path_pattern(&permission.path, path) {
            // Score by specificity: longer patterns and fewer wildcards = more specific
            let specificity = calculate_specificity(&permission.path);

            match &best_match {
                None => best_match = Some((permission, specificity)),
                Some((_, current_score)) if specificity > *current_score => {
                    best_match = Some((permission, specificity));
                }
                _ => {}
            }
        }
    }

    best_match.map(|(p, _)| p)
}

/// Calculate pattern specificity for tie-breaking.
///
/// Higher score = more specific pattern.
fn calculate_specificity(pattern: &str) -> usize {
    let segments: Vec<&str> = pattern.split('.').collect();
    let mut score = 0;

    for segment in &segments {
        match *segment {
            "**" => score += 1, // Recursive wildcard is least specific
            "*" => score += 10, // Single wildcard is more specific
            _ => score += 100,  // Exact match is most specific
        }
    }

    // Bonus for longer patterns
    score += segments.len() * 5;

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::permissions::Operation;
    use std::collections::HashMap;

    fn make_node(path: &str, props: Vec<(&str, &str)>) -> Node {
        let mut properties = HashMap::new();
        for (k, v) in props {
            properties.insert(k.to_string(), PropertyValue::String(v.to_string()));
        }
        Node {
            id: "test".to_string(),
            name: "test".to_string(),
            path: path.to_string(),
            node_type: "test:Type".to_string(),
            properties,
            ..Default::default()
        }
    }

    fn make_permission(
        path: &str,
        fields: Option<Vec<&str>>,
        except: Option<Vec<&str>>,
    ) -> Permission {
        let mut perm = Permission::read_only(path);
        if let Some(f) = fields {
            perm = perm.with_fields(f.into_iter().map(String::from).collect());
        }
        if let Some(e) = except {
            perm = perm.with_except_fields(e.into_iter().map(String::from).collect());
        }
        perm
    }

    #[test]
    fn test_whitelist_filter() {
        let mut node = make_node(
            "/content/article1",
            vec![
                ("title", "Hello"),
                ("content", "World"),
                ("secret", "Password123"),
            ],
        );

        let permission = make_permission("content.**", Some(vec!["title", "content"]), None);
        filter_node_fields(&mut node, &permission);

        assert!(node.properties.contains_key("title"));
        assert!(node.properties.contains_key("content"));
        assert!(!node.properties.contains_key("secret"));
    }

    #[test]
    fn test_blacklist_filter() {
        let mut node = make_node(
            "/content/article1",
            vec![
                ("title", "Hello"),
                ("content", "World"),
                ("internal_notes", "Secret stuff"),
            ],
        );

        let permission = make_permission("content.**", None, Some(vec!["internal_notes"]));
        filter_node_fields(&mut node, &permission);

        assert!(node.properties.contains_key("title"));
        assert!(node.properties.contains_key("content"));
        assert!(!node.properties.contains_key("internal_notes"));
    }

    #[test]
    fn test_whitelist_takes_precedence() {
        let mut node = make_node(
            "/content/article1",
            vec![
                ("title", "Hello"),
                ("content", "World"),
                ("secret", "Password123"),
            ],
        );

        // Both whitelist and blacklist specified - whitelist wins
        let permission = make_permission(
            "content.**",
            Some(vec!["title"]),
            Some(vec!["secret"]), // This is ignored
        );
        filter_node_fields(&mut node, &permission);

        assert!(node.properties.contains_key("title"));
        assert!(!node.properties.contains_key("content")); // Not in whitelist
        assert!(!node.properties.contains_key("secret"));
    }

    #[test]
    fn test_specificity_scoring() {
        assert!(
            calculate_specificity("content.articles.news") > calculate_specificity("content.**")
        );
        assert!(calculate_specificity("content.*.news") > calculate_specificity("content.**"));
        assert!(calculate_specificity("**") < calculate_specificity("content"));
    }
}
