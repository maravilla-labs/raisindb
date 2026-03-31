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

//! Tests for NodeType definition and validation.

#[cfg(test)]
mod tests {
    use crate::nodes::types::node_type::model::definition::{NodeType, NodeTypeVersion};
    use chrono::Utc;
    use serde_json;
    use validator::Validate;

    #[test]
    fn test_nodetype_creation_and_auditable() {
        let node_type = NodeType {
            id: Some("testid".to_string()),
            strict: Some(true),
            name: "test_node".to_string(),
            extends: None,
            mixins: Vec::new(),
            overrides: None,
            description: Some("A test node type".to_string()),
            icon: Some("icon.png".to_string()),
            version: Some(1),
            properties: None,
            allowed_children: Vec::new(),
            required_nodes: Vec::new(),
            initial_structure: None,
            compound_indexes: None,
            versionable: Some(true),
            publishable: Some(true),
            auditable: Some(true),
            indexable: None,
            index_types: None,
            created_at: Some(Utc::now()),
            updated_at: None,
            published_at: None,
            published_by: None,
            previous_version: None,
            is_mixin: None,
        };
        assert!(node_type.auditable());
        let node_type2 = NodeType {
            auditable: None,
            ..node_type.clone()
        };
        assert!(!node_type2.auditable());
    }

    #[test]
    fn test_nodetypeversion_creation() {
        let node_type = NodeType {
            id: Some("testid".to_string()),
            strict: Some(false),
            name: "test_node".to_string(),
            extends: None,
            mixins: Vec::new(),
            overrides: None,
            description: None,
            icon: None,
            version: Some(2),
            properties: None,
            allowed_children: Vec::new(),
            required_nodes: Vec::new(),
            initial_structure: None,
            compound_indexes: None,
            versionable: Some(false),
            publishable: Some(false),
            auditable: Some(false),
            indexable: None,
            index_types: None,
            created_at: Some(Utc::now()),
            updated_at: None,
            published_at: None,
            published_by: None,
            previous_version: None,
            is_mixin: None,
        };
        let version = NodeTypeVersion {
            id: Some("verid".to_string()),
            node_type_id: "testid".to_string(),
            version: 2,
            node_type,
            created_at: Utc::now().to_rfc3339(),
            updated_at: None,
        };
        assert_eq!(version.version, 2);
        assert_eq!(version.node_type_id, "testid");
    }

    #[test]
    fn test_nodetype_serde_json() {
        let node_type = NodeType {
            id: Some("testid".to_string()),
            strict: Some(true),
            name: "test_node".to_string(),
            extends: Some("base".to_string()),
            mixins: vec!["mixin1".to_string()],
            overrides: None,
            description: Some("desc".to_string()),
            icon: Some("icon.png".to_string()),
            version: Some(1),
            properties: None,
            allowed_children: vec!["child1".to_string()],
            required_nodes: Vec::new(),
            initial_structure: None,
            compound_indexes: None,
            versionable: Some(true),
            publishable: Some(false),
            auditable: Some(true),
            indexable: None,
            index_types: None,
            created_at: Some(Utc::now()),
            updated_at: None,
            published_at: None,
            published_by: None,
            previous_version: Some("prev".to_string()),
            is_mixin: None,
        };
        let json = serde_json::to_string(&node_type).expect("Should serialize to JSON");
        let de: NodeType = serde_json::from_str(&json).expect("Should deserialize from JSON");
        assert_eq!(de.name, "test_node");
        assert_eq!(de.extends, Some("base".to_string()));
        assert!(de.auditable());
    }

    #[test]
    fn test_nodetype_validate_pattern() {
        // Valid: namespace:Type format, e.g., raisin:Folder
        let valid = NodeType {
            id: Some("id1".to_string()),
            strict: None,
            name: "raisin:Folder".to_string(),
            extends: Some("standard:Page".to_string()),
            mixins: Vec::new(),
            overrides: None,
            description: None,
            icon: None,
            version: None,
            properties: None,
            allowed_children: Vec::new(),
            required_nodes: Vec::new(),
            initial_structure: None,
            compound_indexes: None,
            versionable: None,
            publishable: None,
            auditable: None,
            indexable: None,
            index_types: None,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            previous_version: None,
            is_mixin: None,
        };
        assert!(valid.validate().is_ok());

        // Invalid: missing namespace or not PascalCase after colon
        let invalid = NodeType {
            id: Some("id2".to_string()),
            strict: None,
            name: "Invalid Name With Spaces".to_string(),
            extends: Some("invalid extends".to_string()),
            mixins: Vec::new(),
            overrides: None,
            description: None,
            icon: None,
            version: None,
            properties: None,
            allowed_children: Vec::new(),
            required_nodes: Vec::new(),
            initial_structure: None,
            compound_indexes: None,
            versionable: None,
            publishable: None,
            auditable: None,
            indexable: None,
            index_types: None,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            previous_version: None,
            is_mixin: None,
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_nodetype_validate_name_valid() {
        let node_type = NodeType::test_minimal("raisin:Folder");
        assert!(node_type.validate().is_ok());
    }

    #[test]
    fn test_nodetype_validate_name_invalid() {
        let node_type = NodeType::test_minimal("Invalid Name With Spaces");
        assert!(node_type.validate().is_err());
    }

    #[test]
    fn test_nodetype_validate_extends_valid() {
        let mut node_type = NodeType::test_minimal("raisin:Folder");
        node_type.extends = Some("standard:Page".to_string());
        assert!(node_type.validate().is_ok());
    }

    #[test]
    fn test_nodetype_validate_extends_invalid() {
        let mut node_type = NodeType::test_minimal("raisin:Folder");
        node_type.extends = Some("invalid extends".to_string());
        assert!(node_type.validate().is_err());
    }

    #[test]
    fn test_nodetype_validate_both_valid() {
        let mut node_type = NodeType::test_minimal("wunder:Page");
        node_type.extends = Some("standard:Page".to_string());
        assert!(node_type.validate().is_ok());
    }

    #[test]
    fn test_nodetype_validate_both_invalid() {
        let mut node_type = NodeType::test_minimal("Invalid Name With Spaces");
        node_type.extends = Some("invalid extends".to_string());
        assert!(node_type.validate().is_err());
    }

    #[tokio::test]
    async fn test_validate_full_no_initial_structure() {
        let node_type = NodeType::test_minimal("test:Simple");

        // Should pass - no initial_structure to validate
        let result = node_type.validate_full(|_| async { Ok(true) }).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_full_with_valid_initial_structure() {
        use crate::nodes::types::initial_structure::{InitialChild, InitialNodeStructure};

        let mut available_types = std::collections::HashSet::new();
        available_types.insert("test:Child".to_string());
        available_types.insert("test:GrandChild".to_string());

        let mut node_type = NodeType::test_minimal("test:Parent");
        node_type.initial_structure = Some(InitialNodeStructure {
            properties: None,
            children: Some(vec![InitialChild {
                name: "child1".to_string(),
                node_type: "test:Child".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: Some(vec![InitialChild {
                    name: "grandchild1".to_string(),
                    node_type: "test:GrandChild".to_string(),
                    archetype: None,
                    properties: None,
                    translations: None,
                    children: None,
                }]),
            }]),
        });

        // Should pass - all referenced types exist
        let result = node_type
            .validate_full(move |name| {
                let available = available_types.clone();
                async move { Ok(available.contains(&name)) }
            })
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_full_with_missing_node_type() {
        use crate::nodes::types::initial_structure::{InitialChild, InitialNodeStructure};

        let mut node_type = NodeType::test_minimal("test:Parent");
        node_type.initial_structure = Some(InitialNodeStructure {
            properties: None,
            children: Some(vec![InitialChild {
                name: "child1".to_string(),
                node_type: "test:MissingType".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: None,
            }]),
        });

        // Should fail - referenced type doesn't exist
        let result = node_type.validate_full(|_| async { Ok(false) }).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Referenced NodeType 'test:MissingType'"));
    }

    #[tokio::test]
    async fn test_validate_full_with_nested_missing_type() {
        use crate::nodes::types::initial_structure::{InitialChild, InitialNodeStructure};

        let mut available_types = std::collections::HashSet::new();
        available_types.insert("test:Child".to_string());
        // Note: test:GrandChild is NOT in the set

        let mut node_type = NodeType::test_minimal("test:Parent");
        node_type.initial_structure = Some(InitialNodeStructure {
            properties: None,
            children: Some(vec![InitialChild {
                name: "child1".to_string(),
                node_type: "test:Child".to_string(),
                archetype: None,
                properties: None,
                translations: None,
                children: Some(vec![InitialChild {
                    name: "grandchild1".to_string(),
                    node_type: "test:GrandChild".to_string(), // Missing!
                    archetype: None,
                    properties: None,
                    translations: None,
                    children: None,
                }]),
            }]),
        });

        // Should fail - nested type doesn't exist
        let result = node_type
            .validate_full(move |name| {
                let available = available_types.clone();
                async move { Ok(available.contains(&name)) }
            })
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Referenced NodeType 'test:GrandChild'"));
    }

    #[test]
    fn test_nodetype_yaml_messagepack_roundtrip() {
        let yaml = include_str!("../../../../../../raisin-core/global_nodetypes/raisin_asset.yaml");

        let node_type: NodeType = serde_yaml::from_str(yaml).expect("YAML should parse");
        assert_eq!(node_type.name, "raisin:Asset");
        assert!(node_type.allowed_children.is_empty());
        assert!(node_type.mixins.is_empty());

        let bytes =
            rmp_serde::to_vec_named(&node_type).expect("MessagePack serialization should work");
        let decoded: NodeType =
            rmp_serde::from_slice(&bytes).expect("MessagePack deserialization should work");

        assert_eq!(decoded.name, "raisin:Asset");
        assert_eq!(decoded.description, node_type.description);
        assert_eq!(decoded.allowed_children, node_type.allowed_children);
        assert_eq!(decoded.properties.as_ref().map(|p| p.len()), Some(5));
    }
}
