// SPDX-License-Identifier: BSL-1.1

//! Content validation types for package type references.

use std::collections::HashSet;

use crate::Manifest;

/// Types that will be available after installation
#[derive(Debug, Clone, Default)]
pub struct AvailableTypes {
    /// Node types
    pub node_types: HashSet<String>,
    /// Mixins (reusable property sets for node types)
    pub mixins: HashSet<String>,
    /// Archetypes
    pub archetypes: HashSet<String>,
    /// Element types
    pub element_types: HashSet<String>,
}

impl AvailableTypes {
    /// Create a new empty set of available types
    pub fn new() -> Self {
        Self::default()
    }

    /// Add types from a package manifest
    pub fn add_from_manifest(&mut self, manifest: &Manifest) {
        // Add node types from provides
        for nt in &manifest.provides.nodetypes {
            self.node_types.insert(nt.clone());
        }
        // Add mixins from provides
        for mixin in &manifest.provides.mixins {
            self.mixins.insert(mixin.clone());
        }
    }

    /// Add a node type
    pub fn add_node_type(&mut self, name: impl Into<String>) {
        self.node_types.insert(name.into());
    }

    /// Add a mixin
    pub fn add_mixin(&mut self, name: impl Into<String>) {
        self.mixins.insert(name.into());
    }

    /// Add an archetype
    pub fn add_archetype(&mut self, name: impl Into<String>) {
        self.archetypes.insert(name.into());
    }

    /// Add an element type
    pub fn add_element_type(&mut self, name: impl Into<String>) {
        self.element_types.insert(name.into());
    }

    /// Check if a node type is available
    pub fn has_node_type(&self, name: &str) -> bool {
        self.node_types.contains(name)
    }

    /// Check if a mixin is available
    pub fn has_mixin(&self, name: &str) -> bool {
        self.mixins.contains(name)
    }

    /// Check if an archetype is available
    pub fn has_archetype(&self, name: &str) -> bool {
        self.archetypes.contains(name)
    }

    /// Check if an element type is available
    pub fn has_element_type(&self, name: &str) -> bool {
        self.element_types.contains(name)
    }

    /// Merge another AvailableTypes into this one
    pub fn merge(&mut self, other: &AvailableTypes) {
        self.node_types.extend(other.node_types.iter().cloned());
        self.mixins.extend(other.mixins.iter().cloned());
        self.archetypes.extend(other.archetypes.iter().cloned());
        self.element_types
            .extend(other.element_types.iter().cloned());
    }
}

/// Validation warning for content type references
#[derive(Debug, Clone)]
pub struct ContentValidationWarning {
    /// File path where the warning occurred
    pub file_path: String,
    /// Type of reference (node_type, archetype, element_type)
    pub reference_type: String,
    /// Name of the missing type
    pub type_name: String,
    /// Message
    pub message: String,
}

impl std::fmt::Display for ContentValidationWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {}: {} '{}' - {}",
            self.file_path, self.reference_type, self.reference_type, self.type_name, self.message
        )
    }
}

/// Result of content validation
#[derive(Debug, Clone, Default)]
pub struct ContentValidationResult {
    /// Warnings (non-fatal issues)
    pub warnings: Vec<ContentValidationWarning>,
    /// Node types referenced in content
    pub referenced_node_types: HashSet<String>,
    /// Mixins referenced in content
    pub referenced_mixins: HashSet<String>,
    /// Archetypes referenced in content
    pub referenced_archetypes: HashSet<String>,
    /// Element types referenced in content
    pub referenced_element_types: HashSet<String>,
}

impl ContentValidationResult {
    /// Create a new empty result
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a warning
    pub fn add_warning(&mut self, warning: ContentValidationWarning) {
        self.warnings.push(warning);
    }

    /// Add a node type reference
    pub fn add_node_type_reference(&mut self, name: impl Into<String>) {
        self.referenced_node_types.insert(name.into());
    }

    /// Add a mixin reference
    pub fn add_mixin_reference(&mut self, name: impl Into<String>) {
        self.referenced_mixins.insert(name.into());
    }

    /// Add an archetype reference
    pub fn add_archetype_reference(&mut self, name: impl Into<String>) {
        self.referenced_archetypes.insert(name.into());
    }

    /// Add an element type reference
    pub fn add_element_type_reference(&mut self, name: impl Into<String>) {
        self.referenced_element_types.insert(name.into());
    }

    /// Check if there are any warnings
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// Merge another result into this one
    pub fn merge(&mut self, other: ContentValidationResult) {
        self.warnings.extend(other.warnings);
        self.referenced_node_types
            .extend(other.referenced_node_types);
        self.referenced_mixins.extend(other.referenced_mixins);
        self.referenced_archetypes
            .extend(other.referenced_archetypes);
        self.referenced_element_types
            .extend(other.referenced_element_types);
    }
}

/// Content validator for packages
///
/// Validates that content references (node_type, archetype, element_type)
/// are available either in the package or in the database.
#[derive(Debug, Clone, Default)]
pub struct ContentValidator {
    /// Types available in the package
    pub package_types: AvailableTypes,
    /// Types available in the database
    pub database_types: AvailableTypes,
}

impl ContentValidator {
    /// Create a new content validator
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the package types (types that will be installed with the package)
    pub fn with_package_types(mut self, types: AvailableTypes) -> Self {
        self.package_types = types;
        self
    }

    /// Set the database types (types already in the database)
    pub fn with_database_types(mut self, types: AvailableTypes) -> Self {
        self.database_types = types;
        self
    }

    /// Check if a node type is available (in package or database)
    pub fn is_node_type_available(&self, name: &str) -> bool {
        self.package_types.has_node_type(name) || self.database_types.has_node_type(name)
    }

    /// Check if a mixin is available (in package or database)
    pub fn is_mixin_available(&self, name: &str) -> bool {
        self.package_types.has_mixin(name) || self.database_types.has_mixin(name)
    }

    /// Check if an archetype is available (in package or database)
    pub fn is_archetype_available(&self, name: &str) -> bool {
        self.package_types.has_archetype(name) || self.database_types.has_archetype(name)
    }

    /// Check if an element type is available (in package or database)
    pub fn is_element_type_available(&self, name: &str) -> bool {
        self.package_types.has_element_type(name) || self.database_types.has_element_type(name)
    }

    /// Validate a mixin reference (e.g., from a node type's mixins field)
    pub fn validate_mixin(
        &self,
        name: &str,
        file_path: &str,
    ) -> Option<ContentValidationWarning> {
        if !self.is_mixin_available(name) {
            Some(ContentValidationWarning {
                file_path: file_path.to_string(),
                reference_type: "mixin".to_string(),
                type_name: name.to_string(),
                message: "Mixin not found in package. It may exist in the database.".to_string(),
            })
        } else {
            None
        }
    }

    /// Validate a node type reference
    pub fn validate_node_type(
        &self,
        name: &str,
        file_path: &str,
    ) -> Option<ContentValidationWarning> {
        if !self.is_node_type_available(name) {
            Some(ContentValidationWarning {
                file_path: file_path.to_string(),
                reference_type: "node_type".to_string(),
                type_name: name.to_string(),
                message: "Node type not found in package. It may exist in the database."
                    .to_string(),
            })
        } else {
            None
        }
    }

    /// Validate an archetype reference
    pub fn validate_archetype(
        &self,
        name: &str,
        file_path: &str,
    ) -> Option<ContentValidationWarning> {
        if !self.is_archetype_available(name) {
            Some(ContentValidationWarning {
                file_path: file_path.to_string(),
                reference_type: "archetype".to_string(),
                type_name: name.to_string(),
                message: "Archetype not found in package. It may exist in the database."
                    .to_string(),
            })
        } else {
            None
        }
    }

    /// Validate an element type reference
    pub fn validate_element_type(
        &self,
        name: &str,
        file_path: &str,
    ) -> Option<ContentValidationWarning> {
        if !self.is_element_type_available(name) {
            Some(ContentValidationWarning {
                file_path: file_path.to_string(),
                reference_type: "element_type".to_string(),
                type_name: name.to_string(),
                message: "Element type not found in package. It may exist in the database."
                    .to_string(),
            })
        } else {
            None
        }
    }
}
