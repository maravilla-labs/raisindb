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

//! Tests for Node core model.

#[cfg(test)]
mod tests {
    use crate::nodes::properties::PropertyValue;
    use crate::nodes::Node;
    use chrono::Utc;
    use std::collections::HashMap;

    fn sample_properties() -> HashMap<String, PropertyValue> {
        let mut props = HashMap::new();
        props.insert(
            "key1".to_string(),
            PropertyValue::String("value1".to_string()),
        );
        props.insert("key2".to_string(), PropertyValue::Integer(42));
        props
    }
    fn sample_node() -> Node {
        Node {
            id: "1".to_string(),
            name: "test_node".to_string(),
            path: "root/child/test_node".to_string(),
            node_type: "test_type".to_string(),
            archetype: Some("text/plain".to_string()),
            properties: sample_properties(),
            children: vec!["child1".to_string(), "child2".to_string()],
            order_key: "a".to_string(),
            has_children: None,
            parent: Some("child".to_string()), // Parent NAME only!
            version: 1,
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
            published_at: None,
            published_by: None,
            updated_by: None,
            created_by: None,
            translations: None,
            tenant_id: None,
            workspace: None,
            owner_id: None,
            relations: Vec::new(),
        }
    }
    #[test]
    fn test_get_parent_full_path() {
        let node = sample_node();
        #[allow(deprecated)]
        let path = node.get_parent_full_path();
        assert_eq!(path, "root/child");
    }
    #[test]
    fn test_get_relative_path_simple() {
        let node = sample_node();
        assert_eq!(node.get_relative_path("sibling"), "sibling");
        assert_eq!(node.get_relative_path("../uncle"), "../uncle");
        assert_eq!(
            node.get_relative_path("../../grandparent"),
            "../../grandparent"
        );
        assert_eq!(node.get_relative_path("./"), "./");
        assert_eq!(node.get_relative_path(""), "./");
    }
    #[test]
    fn test_get_relative_path_absolute() {
        let node = sample_node();
        assert_eq!(node.get_relative_path("/absolute/path"), "/absolute/path");
    }
    #[test]
    fn test_get_relative_path_up_beyond_root() {
        let node = sample_node();
        assert_eq!(node.get_relative_path("../../../foo"), "../../../foo");
    }
    #[test]
    fn test_get_properties() {
        let node = sample_node();
        let props = node.get_properties();
        assert_eq!(
            props.get("key1"),
            Some(&PropertyValue::String("value1".to_string()))
        );
        assert_eq!(props.get("key2"), Some(&PropertyValue::Integer(42)));
        assert!(props.get("key1").is_some());
        assert!(props.get("key2").is_some());
    }
    #[test]
    fn test_json_serialization_deserialization() {
        let node = sample_node();
        let json = serde_json::to_string(&node).expect("Serialization failed");
        let deserialized: Node = serde_json::from_str(&json).expect("Deserialization failed");
        assert_eq!(node.id, deserialized.id);
        assert_eq!(node.name, deserialized.name);
        assert_eq!(node.path, deserialized.path);
        assert_eq!(node.node_type, deserialized.node_type);
        assert_eq!(node.archetype, deserialized.archetype);
        assert_eq!(node.properties, deserialized.properties);
        assert_eq!(node.children, deserialized.children);
        assert_eq!(node.parent, deserialized.parent);
        assert_eq!(node.version, deserialized.version);
        assert_eq!(node.published_at, deserialized.published_at);
        assert_eq!(node.published_by, deserialized.published_by);
        assert_eq!(node.updated_by, deserialized.updated_by);
        assert_eq!(node.created_by, deserialized.created_by);
        assert_eq!(node.translations, deserialized.translations);
        assert_eq!(node.tenant_id, deserialized.tenant_id);
        assert_eq!(node.workspace, deserialized.workspace);
        assert_eq!(node.owner_id, deserialized.owner_id);
        assert!(deserialized.created_at.is_some());
        assert!(deserialized.updated_at.is_some());
    }

    #[test]
    fn test_deserialize_with_null_vec_fields() {
        // Test that Node can be deserialized when Vec fields are explicitly null
        // This simulates SQL query results where array columns may be null
        let json = r#"{
            "id": "test-123",
            "name": "test",
            "path": "/test",
            "node_type": "test:Type",
            "properties": {},
            "children": null,
            "relations": null
        }"#;

        let node: Node =
            serde_json::from_str(json).expect("Node should deserialize with null Vec fields");
        assert_eq!(node.id, "test-123");
        assert!(
            node.children.is_empty(),
            "children should be empty Vec, not fail"
        );
        assert!(
            node.relations.is_empty(),
            "relations should be empty Vec, not fail"
        );
    }

    #[test]
    fn test_deserialize_with_missing_vec_fields() {
        // Test that Node can be deserialized when Vec fields are missing entirely
        let json = r#"{
            "id": "test-456",
            "name": "test2",
            "path": "/test2",
            "node_type": "test:Type2",
            "properties": {}
        }"#;

        let node: Node =
            serde_json::from_str(json).expect("Node should deserialize with missing Vec fields");
        assert_eq!(node.id, "test-456");
        assert!(
            node.children.is_empty(),
            "children should be empty Vec from default"
        );
        assert!(
            node.relations.is_empty(),
            "relations should be empty Vec from default"
        );
    }
}
