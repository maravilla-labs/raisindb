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

//! Breaking change detection for NodeTypes and Workspaces
//!
//! This module provides functions to detect breaking changes between
//! old and new versions of NodeTypes and Workspaces. Breaking changes
//! require explicit confirmation from administrators before being applied.

use raisin_models::nodes::types::NodeType;
use raisin_models::workspace::Workspace;
use raisin_storage::system_updates::{BreakingChange, BreakingChangeType};
use std::collections::HashSet;

/// Detect breaking changes between two NodeType versions
///
/// Breaking changes are modifications that could cause data loss or
/// application errors if applied without careful consideration.
///
/// # Arguments
/// * `old` - The currently applied NodeType
/// * `new` - The new NodeType from the server binary
///
/// # Returns
/// A vector of detected breaking changes (empty if none)
///
/// # Detected Breaking Changes
/// - Property removed
/// - Property type changed
/// - Required constraint added to existing property
/// - Allowed child type removed
/// - Mixin removed
pub fn detect_nodetype_breaking_changes(old: &NodeType, new: &NodeType) -> Vec<BreakingChange> {
    let mut changes = Vec::new();

    // 1. Property removals
    if let (Some(old_props), Some(new_props)) = (&old.properties, &new.properties) {
        let old_names: HashSet<_> = old_props
            .iter()
            .filter_map(|p| p.name.as_ref())
            .cloned()
            .collect();
        let new_names: HashSet<_> = new_props
            .iter()
            .filter_map(|p| p.name.as_ref())
            .cloned()
            .collect();

        for removed in old_names.difference(&new_names) {
            changes.push(BreakingChange {
                change_type: BreakingChangeType::PropertyRemoved,
                description: format!("Property '{}' was removed", removed),
                path: format!("properties.{}", removed),
            });
        }

        // 2. Property type changes
        for old_prop in old_props {
            if let Some(name) = &old_prop.name {
                if let Some(new_prop) = new_props.iter().find(|p| p.name.as_ref() == Some(name)) {
                    // Check if property type changed
                    if old_prop.property_type != new_prop.property_type {
                        changes.push(BreakingChange {
                            change_type: BreakingChangeType::PropertyTypeChanged,
                            description: format!(
                                "Property '{}' type changed from {:?} to {:?}",
                                name, old_prop.property_type, new_prop.property_type
                            ),
                            path: format!("properties.{}.type", name),
                        });
                    }

                    // 3. Required constraint added
                    let old_required = old_prop.required.unwrap_or(false);
                    let new_required = new_prop.required.unwrap_or(false);
                    if !old_required && new_required {
                        changes.push(BreakingChange {
                            change_type: BreakingChangeType::RequiredAdded,
                            description: format!(
                                "Property '{}' is now required (was optional)",
                                name
                            ),
                            path: format!("properties.{}.required", name),
                        });
                    }
                }
            }
        }
    }

    // Handle case where new version has no properties but old did
    if old.properties.is_some() && new.properties.is_none() {
        if let Some(old_props) = &old.properties {
            for prop in old_props {
                if let Some(name) = &prop.name {
                    changes.push(BreakingChange {
                        change_type: BreakingChangeType::PropertyRemoved,
                        description: format!(
                            "Property '{}' was removed (all properties removed)",
                            name
                        ),
                        path: format!("properties.{}", name),
                    });
                }
            }
        }
    }

    // 4. Allowed children removals
    let old_children: HashSet<_> = old.allowed_children.iter().cloned().collect();
    let new_children: HashSet<_> = new.allowed_children.iter().cloned().collect();
    for removed in old_children.difference(&new_children) {
        changes.push(BreakingChange {
            change_type: BreakingChangeType::AllowedChildrenRemoved,
            description: format!("Allowed child type '{}' was removed", removed),
            path: "allowed_children".to_string(),
        });
    }

    // 5. Mixin removals
    let old_mixins: HashSet<_> = old.mixins.iter().cloned().collect();
    let new_mixins: HashSet<_> = new.mixins.iter().cloned().collect();
    for removed in old_mixins.difference(&new_mixins) {
        changes.push(BreakingChange {
            change_type: BreakingChangeType::MixinRemoved,
            description: format!("Mixin '{}' was removed", removed),
            path: "mixins".to_string(),
        });
    }

    changes
}

/// Detect breaking changes between two Workspace versions
///
/// Breaking changes for workspaces include removal of allowed node types
/// or root node types that could prevent existing nodes from being valid.
///
/// # Arguments
/// * `old` - The currently applied Workspace
/// * `new` - The new Workspace from the server binary
///
/// # Returns
/// A vector of detected breaking changes (empty if none)
///
/// # Detected Breaking Changes
/// - Allowed node type removed
/// - Allowed root node type removed
pub fn detect_workspace_breaking_changes(old: &Workspace, new: &Workspace) -> Vec<BreakingChange> {
    let mut changes = Vec::new();

    // 1. Allowed node types removed
    let old_types: HashSet<_> = old.allowed_node_types.iter().cloned().collect();
    let new_types: HashSet<_> = new.allowed_node_types.iter().cloned().collect();
    for removed in old_types.difference(&new_types) {
        changes.push(BreakingChange {
            change_type: BreakingChangeType::AllowedNodeTypeRemoved,
            description: format!("Allowed node type '{}' was removed from workspace", removed),
            path: "allowed_node_types".to_string(),
        });
    }

    // 2. Allowed root node types removed
    let old_root_types: HashSet<_> = old.allowed_root_node_types.iter().cloned().collect();
    let new_root_types: HashSet<_> = new.allowed_root_node_types.iter().cloned().collect();
    for removed in old_root_types.difference(&new_root_types) {
        changes.push(BreakingChange {
            change_type: BreakingChangeType::AllowedRootNodeTypeRemoved,
            description: format!(
                "Allowed root node type '{}' was removed from workspace",
                removed
            ),
            path: "allowed_root_node_types".to_string(),
        });
    }

    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::schema::{PropertyType, PropertyValueSchema as Property};
    use raisin_models::timestamp::StorageTimestamp;
    use raisin_models::workspace::WorkspaceConfig;

    fn create_test_nodetype(name: &str) -> NodeType {
        NodeType {
            id: Some(nanoid::nanoid!()),
            strict: None,
            name: name.to_string(),
            extends: None,
            version: Some(1),
            properties: Some(vec![]),
            allowed_children: vec![],
            required_nodes: vec![],
            mixins: vec![],
            overrides: None,
            description: None,
            icon: None,
            indexable: Some(false),
            index_types: None,
            versionable: Some(false),
            publishable: Some(false),
            auditable: Some(false),
            initial_structure: None,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            previous_version: None,
            compound_indexes: None,
            is_mixin: None,
        }
    }

    fn create_property(name: &str, prop_type: PropertyType) -> Property {
        Property {
            name: Some(name.to_string()),
            property_type: prop_type,
            required: Some(false),
            default: None,
            index: None,
            unique: None,
            constraints: None,
            structure: None,
            items: None,
            value: None,
            meta: None,
            is_translatable: None,
            allow_additional_properties: None,
        }
    }

    #[test]
    fn test_no_breaking_changes() {
        let old = create_test_nodetype("test:Type");
        let new = create_test_nodetype("test:Type");

        let changes = detect_nodetype_breaking_changes(&old, &new);
        assert!(changes.is_empty());
    }

    #[test]
    fn test_property_removed() {
        let mut old = create_test_nodetype("test:Type");
        old.properties = Some(vec![
            create_property("title", PropertyType::String),
            create_property("description", PropertyType::String),
        ]);

        let mut new = create_test_nodetype("test:Type");
        new.properties = Some(vec![create_property("title", PropertyType::String)]);

        let changes = detect_nodetype_breaking_changes(&old, &new);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, BreakingChangeType::PropertyRemoved);
        assert!(changes[0].description.contains("description"));
    }

    #[test]
    fn test_property_type_changed() {
        let mut old = create_test_nodetype("test:Type");
        old.properties = Some(vec![create_property("count", PropertyType::String)]);

        let mut new = create_test_nodetype("test:Type");
        new.properties = Some(vec![create_property("count", PropertyType::Number)]);

        let changes = detect_nodetype_breaking_changes(&old, &new);

        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].change_type,
            BreakingChangeType::PropertyTypeChanged
        );
    }

    #[test]
    fn test_required_added() {
        let mut old = create_test_nodetype("test:Type");
        let mut prop = create_property("title", PropertyType::String);
        prop.required = Some(false);
        old.properties = Some(vec![prop]);

        let mut new = create_test_nodetype("test:Type");
        let mut prop = create_property("title", PropertyType::String);
        prop.required = Some(true);
        new.properties = Some(vec![prop]);

        let changes = detect_nodetype_breaking_changes(&old, &new);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, BreakingChangeType::RequiredAdded);
    }

    #[test]
    fn test_allowed_children_removed() {
        let mut old = create_test_nodetype("test:Type");
        old.allowed_children = vec!["test:Child1".to_string(), "test:Child2".to_string()];

        let mut new = create_test_nodetype("test:Type");
        new.allowed_children = vec!["test:Child1".to_string()];

        let changes = detect_nodetype_breaking_changes(&old, &new);

        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].change_type,
            BreakingChangeType::AllowedChildrenRemoved
        );
    }

    #[test]
    fn test_mixin_removed() {
        let mut old = create_test_nodetype("test:Type");
        old.mixins = vec!["mixin:A".to_string(), "mixin:B".to_string()];

        let mut new = create_test_nodetype("test:Type");
        new.mixins = vec!["mixin:A".to_string()];

        let changes = detect_nodetype_breaking_changes(&old, &new);

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].change_type, BreakingChangeType::MixinRemoved);
    }

    #[test]
    fn test_workspace_allowed_node_type_removed() {
        let old = Workspace {
            name: "test".to_string(),
            description: None,
            allowed_node_types: vec!["type:A".to_string(), "type:B".to_string()],
            allowed_root_node_types: vec![],
            depends_on: vec![],
            config: WorkspaceConfig::default(),
            initial_structure: None,
            created_at: StorageTimestamp::now(),
            updated_at: None,
        };

        let new = Workspace {
            name: "test".to_string(),
            description: None,
            allowed_node_types: vec!["type:A".to_string()],
            allowed_root_node_types: vec![],
            depends_on: vec![],
            config: WorkspaceConfig::default(),
            initial_structure: None,
            created_at: StorageTimestamp::now(),
            updated_at: None,
        };

        let changes = detect_workspace_breaking_changes(&old, &new);

        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].change_type,
            BreakingChangeType::AllowedNodeTypeRemoved
        );
    }

    #[test]
    fn test_adding_properties_is_not_breaking() {
        let mut old = create_test_nodetype("test:Type");
        old.properties = Some(vec![create_property("title", PropertyType::String)]);

        let mut new = create_test_nodetype("test:Type");
        new.properties = Some(vec![
            create_property("title", PropertyType::String),
            create_property("description", PropertyType::String),
        ]);

        let changes = detect_nodetype_breaking_changes(&old, &new);
        assert!(
            changes.is_empty(),
            "Adding properties should not be breaking"
        );
    }

    #[test]
    fn test_adding_allowed_children_is_not_breaking() {
        let mut old = create_test_nodetype("test:Type");
        old.allowed_children = vec!["test:Child1".to_string()];

        let mut new = create_test_nodetype("test:Type");
        new.allowed_children = vec!["test:Child1".to_string(), "test:Child2".to_string()];

        let changes = detect_nodetype_breaking_changes(&old, &new);
        assert!(
            changes.is_empty(),
            "Adding allowed children should not be breaking"
        );
    }
}
