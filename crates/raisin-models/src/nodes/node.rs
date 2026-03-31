pub mod core {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    use crate::nodes::properties::Properties;
    use crate::nodes::properties::PropertyValue;

    fn default_version() -> i32 {
        1
    }

    fn default_string() -> String {
        String::new()
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct Node {
        #[serde(default = "default_string")]
        pub id: String,
        pub name: String,
        #[serde(default = "default_string")]
        pub path: String,
        pub node_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub archetype: Option<String>,
        #[serde(default)]
        pub properties: HashMap<String, PropertyValue>,
        #[serde(default)]
        pub children: Vec<String>,
        pub parent: Option<String>,
        #[serde(default = "default_version")]
        pub version: i32,
        pub created_at: Option<chrono::DateTime<chrono::Utc>>,
        pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub published_at: Option<chrono::DateTime<chrono::Utc>>,
        pub published_by: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub updated_by: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub created_by: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub translations: Option<HashMap<String, PropertyValue>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tenant_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub workspace: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub owner_id: Option<String>,
    }

    impl Node {
        pub fn get_parent_full_path(&self) -> String {
            let split_path: Vec<&str> = self.path.split("/").collect();
            split_path[..split_path.len() - 1].join("/")
        }

        pub fn get_relative_path(&self, target_path: &str) -> String {
            if target_path.starts_with('/') { return target_path.to_string(); }
            let binding = self.get_parent_full_path();
            let current_dir_parts: Vec<&str> = binding.split('/').filter(|s| !s.is_empty()).collect();
            let target_parts: Vec<&str> = target_path.split('/').collect();
            if target_path.is_empty() || target_path == "./" { return "./".to_string(); }
            if target_path.starts_with("../") { return target_path.to_string(); }
            if !target_path.contains("../") { return target_path.to_string(); }
            let mut up_count = 0; for part in &target_parts { if *part == ".." { up_count += 1; } else { break; } }
            let remaining_dirs = if up_count >= current_dir_parts.len() { return target_path.to_string(); } else { current_dir_parts.len() - up_count };
            let prefix = "../".repeat(remaining_dirs); let suffix = target_parts[up_count..].join("/"); format!("{}{}", prefix, suffix)
        }

        pub fn get_properties(&self) -> Properties<'_> { Properties::new(&self.properties) }
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct DeepNode {
        pub node: Node,
        pub children: std::collections::HashMap<String, DeepNode>,
    }

    impl DeepNode { pub fn new(node: Node) -> Self { Self { node, children: Default::default() } } }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::nodes::properties::PropertyValue;
        use chrono::Utc;
        use std::collections::HashMap;

        fn sample_properties() -> HashMap<String, PropertyValue> {
            let mut props = HashMap::new();
            props.insert("key1".to_string(), PropertyValue::String("value1".to_string()));
            props.insert("key2".to_string(), PropertyValue::Integer(42));
            props
        }
        fn sample_node() -> Node { Node { id: "1".to_string(), name: "test_node".to_string(), path: "root/child/test_node".to_string(), node_type: "test_type".to_string(), archetype: Some("text/plain".to_string()), properties: sample_properties(), children: vec!["child1".to_string(), "child2".to_string()], parent: Some("root/child".to_string()), version: 1, created_at: Some(Utc::now()), updated_at: Some(Utc::now()), published_at: None, published_by: None, updated_by: None, created_by: None, translations: None, tenant_id: None, workspace: None, owner_id: None } }
        #[test] fn test_get_parent_full_path() { let node = sample_node(); assert_eq!(node.get_parent_full_path(), "root/child"); }
        #[test] fn test_get_relative_path_simple() { let node = sample_node(); assert_eq!(node.get_relative_path("sibling"), "sibling"); assert_eq!(node.get_relative_path("../uncle"), "../uncle"); assert_eq!(node.get_relative_path("../../grandparent"), "../../grandparent"); assert_eq!(node.get_relative_path("./"), "./"); assert_eq!(node.get_relative_path(""), "./"); }
        #[test] fn test_get_relative_path_absolute() { let node = sample_node(); assert_eq!(node.get_relative_path("/absolute/path"), "/absolute/path"); }
        #[test] fn test_get_relative_path_up_beyond_root() { let node = sample_node(); assert_eq!(node.get_relative_path("../../../foo"), "../../../foo"); }
        #[test] fn test_get_properties() { let node = sample_node(); let props = node.get_properties(); assert_eq!(props.get("key1"), Some(&PropertyValue::String("value1".to_string()))); assert_eq!(props.get("key2"), Some(&PropertyValue::Integer(42))); assert!(props.get("key1").is_some()); assert!(props.get("key2").is_some()); }
        #[test] fn test_json_serialization_deserialization() { let node = sample_node(); let json = serde_json::to_string(&node).expect("Serialization failed"); let deserialized: Node = serde_json::from_str(&json).expect("Deserialization failed"); assert_eq!(node.id, deserialized.id); assert_eq!(node.name, deserialized.name); assert_eq!(node.path, deserialized.path); assert_eq!(node.node_type, deserialized.node_type); assert_eq!(node.archetype, deserialized.archetype); assert_eq!(node.properties, deserialized.properties); assert_eq!(node.children, deserialized.children); assert_eq!(node.parent, deserialized.parent); assert_eq!(node.version, deserialized.version); assert_eq!(node.published_at, deserialized.published_at); assert_eq!(node.published_by, deserialized.published_by); assert_eq!(node.updated_by, deserialized.updated_by); assert_eq!(node.created_by, deserialized.created_by); assert_eq!(node.translations, deserialized.translations); assert_eq!(node.tenant_id, deserialized.tenant_id); assert_eq!(node.workspace, deserialized.workspace); assert_eq!(node.owner_id, deserialized.owner_id); assert!(deserialized.created_at.is_some()); assert!(deserialized.updated_at.is_some()); }

        #[test]
        fn test_minimal_json_deserialization() {
            // This is the minimal JSON that POST API sends
            let json = r#"{"name": "about", "node_type": "page", "properties": {}}"#;
            let result: Result<Node, _> = serde_json::from_str(json);
            if let Err(ref e) = result {
                eprintln!("Deserialization error: {}", e);
            }
            assert!(result.is_ok(), "Failed to deserialize minimal JSON: {:?}", result.err());

            let node = result.unwrap();
            assert_eq!(node.name, "about");
            assert_eq!(node.node_type, "page");
            assert_eq!(node.id, ""); // Should default to empty string
            assert_eq!(node.path, ""); // Should default to empty string
            assert_eq!(node.version, 1); // Should default to 1
            assert!(node.properties.is_empty());
            assert!(node.children.is_empty());
            assert_eq!(node.parent, None);
        }
    }
}

pub use core::*;
