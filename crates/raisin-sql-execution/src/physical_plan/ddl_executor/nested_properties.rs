//! Nested property manipulation helpers for NodeType ALTER operations.
//!
//! These helpers navigate and modify properties at nested paths
//! (e.g., "specs.dimensions.depth") within Object-typed properties.

use raisin_error::Error;
use raisin_models::nodes::properties::schema::PropertyValueSchema;
use raisin_models::nodes::types::node_type::NodeType;

use super::conversions::convert_property;

/// Add a property at a nested path (e.g., "specs.dimensions.depth")
pub(super) fn add_nested_property(
    node_type: &mut NodeType,
    prop_def: &raisin_sql::ast::ddl::PropertyDef,
) -> Result<(), Error> {
    let segments = prop_def.path_segments();
    if segments.len() < 2 {
        return Err(Error::Validation(format!(
            "Invalid nested path: {}",
            prop_def.name
        )));
    }

    let props = node_type.properties.get_or_insert_with(Vec::new);

    // Find the top-level property
    let top_name = segments[0];
    let top_prop = props
        .iter_mut()
        .find(|p| p.name.as_deref() == Some(top_name))
        .ok_or_else(|| {
            Error::Validation(format!("Property '{}' not found at top level", top_name))
        })?;

    // Navigate to the parent structure and add the property
    let leaf_name = prop_def.leaf_name();
    let parent_path = &segments[1..segments.len() - 1];

    let target_structure = navigate_to_structure(top_prop, parent_path)?;

    // Create the new property schema with the leaf name
    let mut new_prop = convert_property(prop_def)?;
    new_prop.name = Some(leaf_name.to_string());

    target_structure.insert(leaf_name.to_string(), new_prop);

    Ok(())
}

/// Drop a property at a nested path (e.g., "specs.dimensions.legacy_field")
pub(super) fn drop_nested_property(node_type: &mut NodeType, path: &str) -> Result<(), Error> {
    let segments: Vec<&str> = path.split('.').collect();
    if segments.len() < 2 {
        return Err(Error::Validation(format!("Invalid nested path: {}", path)));
    }

    let props = node_type
        .properties
        .as_mut()
        .ok_or_else(|| Error::Validation("NodeType has no properties".to_string()))?;

    // Find the top-level property
    let top_name = segments[0];
    let top_prop = props
        .iter_mut()
        .find(|p| p.name.as_deref() == Some(top_name))
        .ok_or_else(|| {
            Error::Validation(format!("Property '{}' not found at top level", top_name))
        })?;

    // Navigate to the parent structure
    let leaf_name = segments
        .last()
        .ok_or_else(|| Error::Validation(format!("Invalid nested path: {}", path)))?;
    let parent_path = &segments[1..segments.len() - 1];

    let target_structure = navigate_to_structure(top_prop, parent_path)?;

    // Remove the property
    target_structure.remove(*leaf_name);

    Ok(())
}

/// Modify a property at a nested path (e.g., "specs.dimensions.width")
pub(super) fn modify_nested_property(
    node_type: &mut NodeType,
    prop_def: &raisin_sql::ast::ddl::PropertyDef,
) -> Result<(), Error> {
    let segments = prop_def.path_segments();
    if segments.len() < 2 {
        return Err(Error::Validation(format!(
            "Invalid nested path: {}",
            prop_def.name
        )));
    }

    let props = node_type
        .properties
        .as_mut()
        .ok_or_else(|| Error::Validation("NodeType has no properties".to_string()))?;

    // Find the top-level property
    let top_name = segments[0];
    let top_prop = props
        .iter_mut()
        .find(|p| p.name.as_deref() == Some(top_name))
        .ok_or_else(|| {
            Error::Validation(format!("Property '{}' not found at top level", top_name))
        })?;

    // Navigate to the parent structure
    let leaf_name = prop_def.leaf_name();
    let parent_path = &segments[1..segments.len() - 1];

    let target_structure = navigate_to_structure(top_prop, parent_path)?;

    // Create the modified property schema with the leaf name
    let mut modified_prop = convert_property(prop_def)?;
    modified_prop.name = Some(leaf_name.to_string());

    // Insert (replaces if exists)
    target_structure.insert(leaf_name.to_string(), modified_prop);

    Ok(())
}

/// Navigate through nested Object structures to reach the target
fn navigate_to_structure<'a>(
    prop: &'a mut PropertyValueSchema,
    path: &[&str],
) -> Result<&'a mut std::collections::HashMap<String, PropertyValueSchema>, Error> {
    let mut current_structure = prop.structure.as_mut().ok_or_else(|| {
        Error::Validation(format!(
            "Property '{}' is not an Object type (has no structure)",
            prop.name.as_deref().unwrap_or("unknown")
        ))
    })?;

    for &segment in path {
        let nested = current_structure
            .get_mut(segment)
            .ok_or_else(|| Error::Validation(format!("Nested property '{}' not found", segment)))?;

        current_structure = nested.structure.as_mut().ok_or_else(|| {
            Error::Validation(format!(
                "Property '{}' is not an Object type (has no structure)",
                segment
            ))
        })?;
    }

    Ok(current_structure)
}
