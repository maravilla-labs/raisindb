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

//! Inheritance resolution for types that support extension hierarchies.
//!
//! This module provides traits and utilities for resolving type inheritance,
//! including detecting circular references and flattening field hierarchies.

use async_trait::async_trait;
use raisin_error::Result;
use raisin_models::nodes::types::element::field_types::FieldSchema;
use std::collections::HashSet;

use crate::errors::ValidationError;

/// Maximum depth for inheritance chains to prevent stack overflow.
///
/// This limit prevents infinite recursion in case circular inheritance
/// is not detected or in pathological inheritance hierarchies.
pub const MAX_INHERITANCE_DEPTH: usize = 20;

/// Trait for types that support inheritance through an `extends` relationship.
///
/// Types implementing this trait can form inheritance hierarchies where
/// child types inherit and potentially override fields from parent types.
pub trait Inheritable {
    /// Get the name of the parent type, if any.
    ///
    /// Returns `Some(parent_name)` if this type extends another type,
    /// or `None` if this is a root type with no parent.
    fn extends(&self) -> Option<&str>;

    /// Get the fields defined directly on this type.
    ///
    /// This does not include inherited fields - only fields defined
    /// at this level of the hierarchy.
    fn fields(&self) -> &[FieldSchema];

    /// Check if this type uses strict mode.
    ///
    /// In strict mode, only fields explicitly defined in the type hierarchy
    /// are allowed. Extra fields cause validation warnings.
    ///
    /// Returns `Some(true)` for strict mode, `Some(false)` for non-strict,
    /// or `None` to inherit from parent or use default behavior.
    fn strict(&self) -> Option<bool>;
}

/// Trait for loading type definitions during inheritance resolution.
///
/// This trait allows different storage backends (database, in-memory cache,
/// etc.) to provide type definitions for the resolution process.
///
/// # Type Parameters
///
/// * `T` - The type being loaded, which must implement `Inheritable`
#[async_trait]
pub trait TypeLoader<T: Inheritable>: Send + Sync {
    /// Load a type definition by name.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the type to load
    ///
    /// # Returns
    ///
    /// * `Ok(Some(type))` if the type exists
    /// * `Ok(None)` if the type does not exist
    /// * `Err(error)` if loading failed
    async fn load(&self, name: &str) -> Result<Option<T>>;
}

/// A type with its inheritance fully resolved.
///
/// This structure contains the original type definition along with
/// computed information from the full inheritance hierarchy.
///
/// # Type Parameters
///
/// * `T` - The type definition, which must implement `Inheritable`
pub struct ResolvedType<T> {
    /// The original type definition
    pub definition: T,

    /// All fields from the complete inheritance hierarchy, with child fields
    /// overriding parent fields of the same name
    pub resolved_fields: Vec<FieldSchema>,

    /// Whether strict mode is enabled (computed from hierarchy)
    pub is_strict: bool,

    /// The complete inheritance chain from root to this type
    /// Example: ["BaseType", "MiddleType", "ThisType"]
    pub inheritance_chain: Vec<String>,
}

impl<T: Inheritable> ResolvedType<T> {
    /// Create a new resolved type from a definition and its resolved data.
    ///
    /// # Arguments
    ///
    /// * `definition` - The original type definition
    /// * `resolved_fields` - Fields from the complete inheritance hierarchy
    /// * `is_strict` - Whether strict mode is enabled
    /// * `inheritance_chain` - The full inheritance chain
    pub fn new(
        definition: T,
        resolved_fields: Vec<FieldSchema>,
        is_strict: bool,
        inheritance_chain: Vec<String>,
    ) -> Self {
        Self {
            definition,
            resolved_fields,
            is_strict,
            inheritance_chain,
        }
    }
}

/// Resolve the complete inheritance hierarchy for a type.
///
/// This function walks the inheritance chain, accumulating fields and resolving
/// strict mode settings. It detects circular inheritance and enforces depth limits.
///
/// # Type Parameters
///
/// * `T` - The type being resolved
/// * `L` - The type loader implementation
///
/// # Arguments
///
/// * `type_def` - The type to resolve
/// * `type_name` - The name of the type (for error reporting)
/// * `loader` - The loader for fetching parent types
///
/// # Returns
///
/// * `Ok(ResolvedType)` with the complete resolved hierarchy
/// * `Err(ValidationError)` if circular inheritance or depth limit exceeded
///
/// # Examples
///
/// ```rust,ignore
/// use raisin_validation::{resolve_inheritance, TypeLoader};
///
/// let resolved = resolve_inheritance(element_type, "Article", &loader).await?;
/// println!("Resolved {} fields", resolved.resolved_fields.len());
/// ```
pub async fn resolve_inheritance<T, L>(
    type_def: T,
    type_name: &str,
    loader: &L,
) -> Result<ResolvedType<T>>
where
    T: Inheritable,
    L: TypeLoader<T>,
{
    let mut visited = HashSet::new();
    let mut chain = Vec::new();
    let mut all_fields = Vec::new();
    let mut is_strict = type_def.strict();

    // Start with the current type
    visited.insert(type_name.to_string());
    chain.push(type_name.to_string());

    // Add fields from current type
    all_fields.extend(type_def.fields().iter().cloned());

    // Walk up the inheritance chain
    let mut current_parent = type_def.extends().map(|s| s.to_string());
    let mut depth = 0;

    while let Some(parent_name) = current_parent {
        depth += 1;

        // Check depth limit
        if depth > MAX_INHERITANCE_DEPTH {
            return Err(
                ValidationError::max_inheritance_depth(type_name, MAX_INHERITANCE_DEPTH).into(),
            );
        }

        // Check for circular inheritance
        if visited.contains(&parent_name) {
            return Err(ValidationError::circular_inheritance(type_name, &chain).into());
        }

        // Load parent type
        let parent = loader
            .load(&parent_name)
            .await?
            .ok_or_else(|| ValidationError::unknown_element_type(type_name, &parent_name))?;

        // Track this parent
        visited.insert(parent_name.clone());
        chain.push(parent_name.clone());

        // Merge parent fields (current fields override parent fields with same name)
        let parent_fields = parent.fields();
        for parent_field in parent_fields {
            let field_name = crate::field_helpers::field_name(parent_field);
            // Only add parent field if not already defined by child
            if !all_fields
                .iter()
                .any(|f| crate::field_helpers::field_name(f) == field_name)
            {
                all_fields.push(parent_field.clone());
            }
        }

        // Inherit strict mode if not set at current level
        if is_strict.is_none() {
            is_strict = parent.strict();
        }

        // Move to next parent
        current_parent = parent.extends().map(|s| s.to_string());
    }

    // Reverse chain so it goes from root to current type
    chain.reverse();

    Ok(ResolvedType::new(
        type_def,
        all_fields,
        is_strict.unwrap_or(false),
        chain,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::types::element::fields::base_field::FieldTypeSchema;

    // Test type implementing Inheritable
    #[derive(Debug, Clone)]
    struct TestType {
        name: String,
        extends: Option<String>,
        fields: Vec<FieldSchema>,
        strict: Option<bool>,
    }

    impl Inheritable for TestType {
        fn extends(&self) -> Option<&str> {
            self.extends.as_deref()
        }

        fn fields(&self) -> &[FieldSchema] {
            &self.fields
        }

        fn strict(&self) -> Option<bool> {
            self.strict
        }
    }

    // Test loader using in-memory map
    struct TestLoader {
        types: std::collections::HashMap<String, TestType>,
    }

    #[async_trait]
    impl TypeLoader<TestType> for TestLoader {
        async fn load(&self, name: &str) -> Result<Option<TestType>> {
            Ok(self.types.get(name).cloned())
        }
    }

    fn create_field(name: &str) -> FieldSchema {
        FieldSchema::TextField {
            base: FieldTypeSchema {
                name: name.to_string(),
                ..Default::default()
            },
            config: None,
        }
    }

    #[tokio::test]
    async fn test_simple_inheritance() {
        let mut types = std::collections::HashMap::new();

        // Base type
        types.insert(
            "Base".to_string(),
            TestType {
                name: "Base".to_string(),
                extends: None,
                fields: vec![create_field("base_field")],
                strict: None,
            },
        );

        let loader = TestLoader { types };

        // Child type extending Base
        let child = TestType {
            name: "Child".to_string(),
            extends: Some("Base".to_string()),
            fields: vec![create_field("child_field")],
            strict: None,
        };

        let resolved = resolve_inheritance(child, "Child", &loader)
            .await
            .expect("Resolution should succeed");

        assert_eq!(resolved.resolved_fields.len(), 2);
        assert_eq!(resolved.inheritance_chain, vec!["Base", "Child"]);
    }

    #[tokio::test]
    async fn test_field_override() {
        let mut types = std::collections::HashMap::new();

        // Base type
        types.insert(
            "Base".to_string(),
            TestType {
                name: "Base".to_string(),
                extends: None,
                fields: vec![create_field("shared_field")],
                strict: None,
            },
        );

        let loader = TestLoader { types };

        // Child type with same field name (should override)
        let child = TestType {
            name: "Child".to_string(),
            extends: Some("Base".to_string()),
            fields: vec![create_field("shared_field")],
            strict: None,
        };

        let resolved = resolve_inheritance(child, "Child", &loader)
            .await
            .expect("Resolution should succeed");

        // Should have only 1 field (child's version)
        assert_eq!(resolved.resolved_fields.len(), 1);
        assert_eq!(
            crate::field_helpers::field_name(&resolved.resolved_fields[0]),
            "shared_field"
        );
    }

    #[tokio::test]
    async fn test_circular_inheritance() {
        let mut types = std::collections::HashMap::new();

        // Create circular reference: A -> B -> A
        types.insert(
            "TypeA".to_string(),
            TestType {
                name: "TypeA".to_string(),
                extends: Some("TypeB".to_string()),
                fields: vec![],
                strict: None,
            },
        );

        types.insert(
            "TypeB".to_string(),
            TestType {
                name: "TypeB".to_string(),
                extends: Some("TypeA".to_string()),
                fields: vec![],
                strict: None,
            },
        );

        let loader = TestLoader {
            types: types.clone(),
        };

        let type_a = types.get("TypeA").unwrap().clone();
        let result = resolve_inheritance(type_a, "TypeA", &loader).await;

        assert!(result.is_err());
    }
}
