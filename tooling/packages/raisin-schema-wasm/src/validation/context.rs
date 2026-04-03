//! Validation context, regex patterns, and valid property types

use crate::builtin_types::{is_builtin_node_type, is_builtin_workspace};
use lazy_static::lazy_static;
use regex::Regex;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::Archetype;
use std::collections::{HashMap, HashSet};

lazy_static! {
    /// Regex for valid node type names: namespace:PascalCase
    /// e.g., "raisin:Folder", "custom:MyType"
    pub(crate) static ref NODE_TYPE_NAME_REGEX: Regex =
        Regex::new(r"^[a-zA-Z]+:(?:[A-Z][a-z0-9]*)+$").unwrap();

    /// Regex for valid package names: alphanumeric, hyphens, underscores
    pub(crate) static ref PACKAGE_NAME_REGEX: Regex =
        Regex::new(r"^[a-zA-Z][a-zA-Z0-9_-]*$").unwrap();

    /// Valid property types
    pub(crate) static ref VALID_PROPERTY_TYPES: HashSet<&'static str> = {
        let mut s = HashSet::new();
        s.insert("String");
        s.insert("string");
        s.insert("Number");
        s.insert("number");
        s.insert("Boolean");
        s.insert("boolean");
        s.insert("Array");
        s.insert("array");
        s.insert("Object");
        s.insert("object");
        s.insert("Date");
        s.insert("date");
        s.insert("URL");
        s.insert("url");
        s.insert("Reference");
        s.insert("reference");
        s.insert("NodeType");
        s.insert("nodetype");
        s.insert("nodeType");
        s.insert("Element");
        s.insert("element");
        s.insert("Composite");
        s.insert("composite");
        s.insert("Resource");
        s.insert("resource");
        s.insert("Geometry");
        s.insert("geometry");
        s
    };
}

/// Context for validation containing known types from the package
pub struct ValidationContext {
    /// Node types defined in the current package
    pub package_node_types: HashSet<String>,
    /// Workspaces defined in the current package
    pub package_workspaces: HashSet<String>,
    /// Archetypes defined in the current package (full definitions for field validation)
    pub package_archetypes: HashMap<String, Archetype>,
    /// Element types defined in the current package (full definitions for field validation)
    pub package_element_types: HashMap<String, ElementType>,
    /// Content node paths for cross-reference validation: (workspace, lowercase_path) → actual_path
    pub content_node_paths: HashMap<(String, String), String>,
}

impl Default for ValidationContext {
    fn default() -> Self {
        Self {
            package_node_types: HashSet::new(),
            package_workspaces: HashSet::new(),
            package_archetypes: HashMap::new(),
            package_element_types: HashMap::new(),
            content_node_paths: HashMap::new(),
        }
    }
}

impl ValidationContext {
    /// Check if a node type reference is valid
    pub fn is_valid_node_type_ref(&self, name: &str) -> bool {
        is_builtin_node_type(name) || self.package_node_types.contains(name)
    }

    /// Check if a workspace reference is valid
    pub fn is_valid_workspace_ref(&self, name: &str) -> bool {
        is_builtin_workspace(name) || self.package_workspaces.contains(name)
    }

    /// Check if an archetype reference is valid
    pub fn is_valid_archetype_ref(&self, name: &str) -> bool {
        self.package_archetypes.contains_key(name)
    }

    /// Check if an element type reference is valid
    pub fn is_valid_element_type_ref(&self, name: &str) -> bool {
        self.package_element_types.contains_key(name)
    }

    /// Get an archetype definition by name
    pub fn get_archetype(&self, name: &str) -> Option<&Archetype> {
        self.package_archetypes.get(name)
    }

    /// Get an element type definition by name
    pub fn get_element_type(&self, name: &str) -> Option<&ElementType> {
        self.package_element_types.get(name)
    }
}
