//! Element type and field validation.
//!
//! Validates element types, section fields, and composite/array property values
//! against resolved ElementType schemas. Supports recursive validation of nested
//! elements and implicit element detection in Object values.

use async_recursion::async_recursion;
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::value::{Element, PropertyValue};
use raisin_models::nodes::types::element::field_types::FieldSchema;
use raisin_models::nodes::Node;
use raisin_storage::Storage;
use raisin_validation::{field_name, is_multiple, is_required};
use std::collections::{HashMap, HashSet};

use crate::services::element_type_resolver::ResolvedElementType;

use super::core::NodeValidator;

impl<S: Storage> NodeValidator<S> {
    /// Validate all element types found in node properties
    pub(super) async fn validate_element_types(
        &self,
        node: &Node,
        cache: &mut HashMap<String, ResolvedElementType>,
    ) -> Result<()> {
        for (property_name, value) in &node.properties {
            self.validate_property_value_recursive(value, property_name, cache)
                .await?;
        }

        Ok(())
    }

    /// Validate property values against a field schema list
    #[async_recursion]
    pub(super) async fn validate_fields_against_schema(
        &self,
        values: &HashMap<String, PropertyValue>,
        fields: &[FieldSchema],
        path: &str,
        cache: &mut HashMap<String, ResolvedElementType>,
    ) -> Result<()> {
        for field in fields {
            let name = field_name(field);
            let value = values.get(name);

            if is_required(field) && value.is_none() {
                return Err(Error::Validation(format!(
                    "Missing required field '{}' at {}",
                    name, path
                )));
            }

            if let Some(value) = value {
                let field_path = format!("{}.{}", path, name);
                self.validate_field_value(field, value, &field_path, cache)
                    .await?;
            }
        }

        Ok(())
    }

    /// Validate a single field value based on field schema type
    #[async_recursion]
    async fn validate_field_value(
        &self,
        field: &FieldSchema,
        value: &PropertyValue,
        path: &str,
        cache: &mut HashMap<String, ResolvedElementType>,
    ) -> Result<()> {
        match field {
            FieldSchema::CompositeField { .. } => {
                // When multiple + translatable sub-fields, require unique UUIDs
                if raisin_validation::composite_requires_uuid(field) {
                    if let PropertyValue::Array(items) = value {
                        let mut seen = HashSet::new();
                        for (idx, item) in items.iter().enumerate() {
                            if let PropertyValue::Object(obj) = item {
                                match obj.get("uuid") {
                                    None => {
                                        return Err(Error::Validation(format!(
                                            "COMPOSITE_MISSING_UUID: Item {}[{}] requires a 'uuid' field because the composite has translatable sub-fields",
                                            path, idx
                                        )));
                                    }
                                    Some(PropertyValue::String(u))
                                        if !seen.insert(u.clone()) =>
                                    {
                                        return Err(Error::Validation(format!(
                                            "COMPOSITE_DUPLICATE_UUID: Duplicate uuid '{}' in composite at {}[{}]",
                                            u, path, idx
                                        )));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }

                self.validate_property_value_recursive(value, path, cache)
                    .await
            }
            FieldSchema::ElementField { element_type, .. } => {
                self.validate_element_field_value(
                    element_type,
                    value,
                    is_multiple(field),
                    path,
                    cache,
                )
                .await
            }
            FieldSchema::SectionField {
                allowed_element_types,
                ..
            } => {
                self.validate_section_field_value(
                    allowed_element_types.as_ref(),
                    value,
                    path,
                    cache,
                )
                .await
            }
            _ => {
                self.validate_property_value_recursive(value, path, cache)
                    .await
            }
        }
    }

    /// Validate an element field value against its expected element type
    #[async_recursion]
    async fn validate_element_field_value(
        &self,
        expected_type: &str,
        value: &PropertyValue,
        allow_multiple: bool,
        path: &str,
        cache: &mut HashMap<String, ResolvedElementType>,
    ) -> Result<()> {
        match value {
            PropertyValue::Element(element) => {
                if element.element_type != expected_type {
                    return Err(Error::Validation(format!(
                        "Field '{}' expects element type '{}', found '{}'",
                        path, expected_type, element.element_type
                    )));
                }
                self.validate_element_instance(element, path, cache).await
            }
            PropertyValue::Composite(composite) => {
                if !allow_multiple && composite.items.len() > 1 {
                    return Err(Error::Validation(format!(
                        "Field '{}' does not allow multiple elements",
                        path
                    )));
                }
                for (idx, element) in composite.items.iter().enumerate() {
                    if element.element_type != expected_type {
                        return Err(Error::Validation(format!(
                            "Field '{}[{}]' expects element type '{}', found '{}'",
                            path, idx, expected_type, element.element_type
                        )));
                    }
                    let nested_path = format!("{path}[{idx}]");
                    self.validate_element_instance(element, &nested_path, cache)
                        .await?;
                }
                Ok(())
            }
            PropertyValue::Array(items) => {
                if !allow_multiple && items.len() > 1 {
                    return Err(Error::Validation(format!(
                        "Field '{}' does not allow multiple elements",
                        path
                    )));
                }
                for (idx, item) in items.iter().enumerate() {
                    match item {
                        PropertyValue::Element(element) => {
                            if element.element_type != expected_type {
                                return Err(Error::Validation(format!(
                                    "Field '{}[{}]' expects element type '{}', found '{}'",
                                    path, idx, expected_type, element.element_type
                                )));
                            }
                            let nested_path = format!("{path}[{idx}]");
                            self.validate_element_instance(element, &nested_path, cache)
                                .await?;
                        }
                        _ => {
                            return Err(Error::Validation(format!(
                                "Field '{}[{}]' expects element values",
                                path, idx
                            )));
                        }
                    }
                }
                Ok(())
            }
            _ => Err(Error::Validation(format!(
                "Field '{}' expects element type '{}'",
                path, expected_type
            ))),
        }
    }

    /// Validate a section field value against allowed element types
    #[async_recursion]
    async fn validate_section_field_value(
        &self,
        allowed_types: Option<&Vec<String>>,
        value: &PropertyValue,
        path: &str,
        cache: &mut HashMap<String, ResolvedElementType>,
    ) -> Result<()> {
        match value {
            PropertyValue::Composite(composite) => {
                for (idx, element) in composite.items.iter().enumerate() {
                    if let Some(list) = allowed_types {
                        if !list.iter().any(|name| name == &element.element_type) {
                            return Err(Error::Validation(format!(
                                "Element type '{}' is not allowed in field '{}'",
                                element.element_type, path
                            )));
                        }
                    }
                    let nested_path = format!("{path}[{idx}]");
                    self.validate_element_instance(element, &nested_path, cache)
                        .await?;
                }
                Ok(())
            }
            PropertyValue::Array(items) => {
                for (idx, item) in items.iter().enumerate() {
                    match item {
                        PropertyValue::Element(element) => {
                            if let Some(list) = allowed_types {
                                if !list.iter().any(|name| name == &element.element_type) {
                                    return Err(Error::Validation(format!(
                                        "Element type '{}' is not allowed in field '{}'",
                                        element.element_type, path
                                    )));
                                }
                            }
                            let nested_path = format!("{path}[{idx}]");
                            self.validate_element_instance(element, &nested_path, cache)
                                .await?;
                        }
                        _ => {
                            return Err(Error::Validation(format!(
                                "Field '{}[{}]' expects element values",
                                path, idx
                            )));
                        }
                    }
                }
                Ok(())
            }
            PropertyValue::Element(element) => {
                if let Some(list) = allowed_types {
                    if !list.iter().any(|name| name == &element.element_type) {
                        return Err(Error::Validation(format!(
                            "Element type '{}' is not allowed in field '{}'",
                            element.element_type, path
                        )));
                    }
                }
                self.validate_element_instance(element, path, cache).await
            }
            _ => {
                self.validate_property_value_recursive(value, path, cache)
                    .await
            }
        }
    }

    /// Recursively validate property values, detecting nested elements
    #[async_recursion]
    pub(super) async fn validate_property_value_recursive(
        &self,
        value: &PropertyValue,
        path: &str,
        cache: &mut HashMap<String, ResolvedElementType>,
    ) -> Result<()> {
        match value {
            PropertyValue::Element(element) => {
                self.validate_element_instance(element, path, cache).await
            }
            PropertyValue::Composite(composite) => {
                for (idx, element) in composite.items.iter().enumerate() {
                    let nested_path = format!("{path}[{idx}]");
                    self.validate_element_instance(element, &nested_path, cache)
                        .await?;
                }
                Ok(())
            }
            PropertyValue::Array(items) => {
                for (idx, item) in items.iter().enumerate() {
                    let nested_path = format!("{path}[{idx}]");
                    self.validate_property_value_recursive(item, &nested_path, cache)
                        .await?;
                }
                Ok(())
            }
            PropertyValue::Object(entries) => {
                // Check if this Object is actually an implicit Element (has element_type + content)
                // This happens when data lacks 'uuid' - serde falls back to Object instead of Element
                if let (Some(PropertyValue::String(element_type)), Some(content_value)) =
                    (entries.get("element_type"), entries.get("content"))
                {
                    // Extract content as HashMap for element validation
                    if let PropertyValue::Object(content_map) = content_value {
                        // Create a temporary Element for validation
                        let implicit_element = Element {
                            uuid: entries
                                .get("uuid")
                                .and_then(|v| match v {
                                    PropertyValue::String(s) => Some(s.clone()),
                                    _ => None,
                                })
                                .unwrap_or_default(),
                            element_type: element_type.clone(),
                            content: content_map.clone(),
                        };
                        self.validate_element_instance(&implicit_element, path, cache)
                            .await?;
                    }

                    // Also recursively validate nested content for further nested elements
                    for (key, item) in entries {
                        if key != "element_type" && key != "uuid" {
                            let nested_path = format!("{path}.{}", key);
                            self.validate_property_value_recursive(item, &nested_path, cache)
                                .await?;
                        }
                    }
                } else {
                    // Regular object - just validate nested values
                    for (key, item) in entries {
                        let nested_path = format!("{path}.{}", key);
                        self.validate_property_value_recursive(item, &nested_path, cache)
                            .await?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Validate a single element instance against its resolved ElementType schema
    #[async_recursion]
    async fn validate_element_instance(
        &self,
        element: &Element,
        path: &str,
        cache: &mut HashMap<String, ResolvedElementType>,
    ) -> Result<()> {
        let resolved = match self.fetch_element_type(&element.element_type, cache).await {
            Ok(resolved) => resolved,
            Err(Error::Validation(message)) => {
                return Err(Error::Validation(format!(
                    "{} (at element '{}', path '{}')",
                    message, element.element_type, path
                )))
            }
            Err(other) => return Err(other),
        };

        // Validate against resolved fields (includes inherited fields from parent element types)
        self.validate_fields_against_schema(
            &element.content,
            &resolved.resolved_fields,
            path,
            cache,
        )
        .await?;

        // Check strict mode for element type (no undefined properties allowed)
        if resolved.resolved_strict {
            self.check_element_strict_mode(element, &resolved, path)?;
        }

        Ok(())
    }

    /// Check that no undefined properties exist for element (strict mode)
    fn check_element_strict_mode(
        &self,
        element: &Element,
        resolved: &ResolvedElementType,
        path: &str,
    ) -> Result<()> {
        // Build set of allowed field names from resolved element type
        let allowed_fields: HashSet<&str> =
            resolved.resolved_fields.iter().map(field_name).collect();

        // Check each element property against allowed fields
        for key in element.content.keys() {
            if !allowed_fields.contains(key.as_str()) {
                return Err(Error::Validation(format!(
                    "Undefined property '{}' in strict element type '{}' at path '{}'",
                    key, resolved.element_type.name, path
                )));
            }
        }

        Ok(())
    }

    /// Fetch and cache a resolved element type
    pub(super) async fn fetch_element_type(
        &self,
        name: &str,
        cache: &mut HashMap<String, ResolvedElementType>,
    ) -> Result<ResolvedElementType> {
        if let Some(existing) = cache.get(name) {
            return Ok(existing.clone());
        }

        // Use resolver to get element type with full inheritance chain
        let resolved = self
            .element_type_resolver
            .resolve(name)
            .await
            .map_err(|e| {
                Error::Validation(format!("Failed to resolve element type '{}': {}", name, e))
            })?;

        cache.insert(name.to_string(), resolved.clone());
        Ok(resolved)
    }
}
