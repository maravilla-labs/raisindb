// NodeType struct, NodeTypeVersion, and main impl

use crate::nodes::properties::schema::{CompoundIndexDefinition, PropertyValueSchema};
use crate::nodes::properties::value::PropertyValue;
use chrono::{DateTime, Utc};
use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use validator::Validate;

pub type OverrideProperties = HashMap<String, PropertyValue>;

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Validate)]
pub struct NodeType {
    #[serde(default = "default_uuid")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
    #[validate(regex(path = "*crate::nodes::types::utils::URL_FRIENDLY_NAME_REGEX"))]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[validate(regex(path = "*crate::nodes::types::utils::URL_FRIENDLY_NAME_REGEX"))]
    pub extends: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mixins: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overrides: Option<OverrideProperties>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub version: Option<i32>,
    pub properties: Option<Vec<PropertyValueSchema>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_children: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_nodes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_structure: Option<super::initial_structure::InitialNodeStructure>,
    #[serde(default)]
    pub versionable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publishable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auditable: Option<bool>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<String>,
    /// Compound indexes for efficient multi-column queries
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compound_indexes: Option<Vec<CompoundIndexDefinition>>,
}

fn default_uuid() -> Option<String> {
    Some(nanoid!(16))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodeTypeVersion {
    pub id: Option<String>,
    pub node_type_id: String,
    pub version: i32,
    pub node_type: NodeType,
    pub created_at: String,
    pub updated_at: Option<DateTime<Utc>>,
}

impl NodeType {
    pub fn auditable(&self) -> bool {
        self.auditable.unwrap_or(false)
    }

    pub fn is_published(&self) -> bool {
        self.publishable.unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            mixins: None,
            overrides: None,
            description: Some("A test node type".to_string()),
            icon: Some("icon.png".to_string()),
            version: Some(1),
            properties: None,
            allowed_children: None,
            required_nodes: None,
            initial_structure: None,
            versionable: Some(true),
            publishable: Some(true),
            auditable: Some(true),
            created_at: Some(Utc::now()),
            updated_at: None,
            previous_version: None,
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
            mixins: None,
            overrides: None,
            description: None,
            icon: None,
            version: Some(2),
            properties: None,
            allowed_children: None,
            required_nodes: None,
            initial_structure: None,
            versionable: Some(false),
            publishable: Some(false),
            auditable: Some(false),
            created_at: Some(Utc::now()),
            updated_at: None,
            previous_version: None,
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
            mixins: Some(vec!["mixin1".to_string()]),
            overrides: None,
            description: Some("desc".to_string()),
            icon: Some("icon.png".to_string()),
            version: Some(1),
            properties: None,
            allowed_children: Some(vec!["child1".to_string()]),
            required_nodes: None,
            initial_structure: None,
            versionable: Some(true),
            publishable: Some(false),
            auditable: Some(true),
            created_at: Some(Utc::now()),
            updated_at: None,
            previous_version: Some("prev".to_string()),
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
            mixins: None,
            overrides: None,
            description: None,
            icon: None,
            version: None,
            properties: None,
            allowed_children: None,
            required_nodes: None,
            initial_structure: None,
            versionable: None,
            publishable: None,
            auditable: None,
            created_at: None,
            updated_at: None,
            previous_version: None,
        };
        assert!(valid.validate().is_ok());

        // Invalid: missing namespace or not PascalCase after colon
        let invalid = NodeType {
            id: Some("id2".to_string()),
            strict: None,
            name: "Invalid Name With Spaces".to_string(),
            extends: Some("invalid extends".to_string()),
            mixins: None,
            overrides: None,
            description: None,
            icon: None,
            version: None,
            properties: None,
            allowed_children: None,
            required_nodes: None,
            initial_structure: None,
            versionable: None,
            publishable: None,
            auditable: None,
            created_at: None,
            updated_at: None,
            previous_version: None,
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn test_nodetype_validate_name_valid() {
        let node_type = NodeType {
            id: Some("id1".to_string()),
            strict: None,
            name: "raisin:Folder".to_string(),
            extends: None,
            mixins: None,
            overrides: None,
            description: None,
            icon: None,
            version: None,
            properties: None,
            allowed_children: None,
            required_nodes: None,
            initial_structure: None,
            versionable: None,
            publishable: None,
            auditable: None,
            created_at: None,
            updated_at: None,
            previous_version: None,
        };
        assert!(node_type.validate().is_ok());
    }

    #[test]
    fn test_nodetype_validate_name_invalid() {
        let node_type = NodeType {
            id: Some("id2".to_string()),
            strict: None,
            name: "Invalid Name With Spaces".to_string(),
            extends: None,
            mixins: None,
            overrides: None,
            description: None,
            icon: None,
            version: None,
            properties: None,
            allowed_children: None,
            required_nodes: None,
            initial_structure: None,
            versionable: None,
            publishable: None,
            auditable: None,
            created_at: None,
            updated_at: None,
            previous_version: None,
        };
        assert!(node_type.validate().is_err());
    }

    #[test]
    fn test_nodetype_validate_extends_valid() {
        let node_type = NodeType {
            id: Some("id3".to_string()),
            strict: None,
            name: "raisin:Folder".to_string(),
            extends: Some("standard:Page".to_string()),
            mixins: None,
            overrides: None,
            description: None,
            icon: None,
            version: None,
            properties: None,
            allowed_children: None,
            required_nodes: None,
            initial_structure: None,
            versionable: None,
            publishable: None,
            auditable: None,
            created_at: None,
            updated_at: None,
            previous_version: None,
        };
        assert!(node_type.validate().is_ok());
    }

    #[test]
    fn test_nodetype_validate_extends_invalid() {
        let node_type = NodeType {
            id: Some("id4".to_string()),
            strict: None,
            name: "raisin:Folder".to_string(),
            extends: Some("invalid extends".to_string()),
            mixins: None,
            overrides: None,
            description: None,
            icon: None,
            version: None,
            properties: None,
            allowed_children: None,
            required_nodes: None,
            initial_structure: None,
            versionable: None,
            publishable: None,
            auditable: None,
            created_at: None,
            updated_at: None,
            previous_version: None,
        };
        assert!(node_type.validate().is_err());
    }

    #[test]
    fn test_nodetype_validate_both_valid() {
        let node_type = NodeType {
            id: Some("id5".to_string()),
            strict: None,
            name: "wunder:Page".to_string(),
            extends: Some("standard:Page".to_string()),
            mixins: None,
            overrides: None,
            description: None,
            icon: None,
            version: None,
            properties: None,
            allowed_children: None,
            required_nodes: None,
            initial_structure: None,
            versionable: None,
            publishable: None,
            auditable: None,
            created_at: None,
            updated_at: None,
            previous_version: None,
        };
        assert!(node_type.validate().is_ok());
    }

    #[test]
    fn test_nodetype_validate_both_invalid() {
        let node_type = NodeType {
            id: Some("id6".to_string()),
            strict: None,
            name: "Invalid Name With Spaces".to_string(),
            extends: Some("invalid extends".to_string()),
            mixins: None,
            overrides: None,
            description: None,
            icon: None,
            version: None,
            properties: None,
            allowed_children: None,
            required_nodes: None,
            initial_structure: None,
            versionable: None,
            publishable: None,
            auditable: None,
            created_at: None,
            updated_at: None,
            previous_version: None,
        };
        assert!(node_type.validate().is_err());
    }
}
