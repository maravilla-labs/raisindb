// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Schema type builders and assignment applicators.
//!
//! Contains functions to build NodeType, Archetype, and ElementType models
//! from column maps, and to apply individual field assignments to them.

use indexmap::IndexMap;
use raisin_error::Error;
use raisin_models::nodes::{
    properties::PropertyValue,
    types::{archetype::Archetype, element::element_type::ElementType, NodeType},
};

use super::helpers::{
    convert_property_value, extract_boolean_value, extract_number_value,
    extract_optional_boolean_column, extract_optional_string_column, extract_string_array,
    extract_string_column, extract_string_value,
};

/// Build a NodeType from column->value map.
pub(super) fn build_nodetype_from_columns(
    col_map: &IndexMap<String, PropertyValue>,
) -> Result<NodeType, Error> {
    let id = extract_optional_string_column(col_map, "id");
    let name = extract_string_column(col_map, "name")?;
    let version = col_map.get("version").and_then(|v| match v {
        PropertyValue::Integer(n) => Some(*n as i32),
        PropertyValue::Float(n) => Some(*n as i32),
        _ => None,
    });

    let description = extract_optional_string_column(col_map, "description");
    let strict = extract_optional_boolean_column(col_map, "strict");
    let versionable = extract_optional_boolean_column(col_map, "versionable");
    let publishable = extract_optional_boolean_column(col_map, "publishable");
    let auditable = extract_optional_boolean_column(col_map, "auditable");
    let indexable = extract_optional_boolean_column(col_map, "indexable");

    let mixins = if let Some(val) = col_map.get("mixins") {
        extract_string_array(val)?
    } else {
        Vec::new()
    };
    let allowed_children = if let Some(val) = col_map.get("allowed_children") {
        extract_string_array(val)?
    } else {
        Vec::new()
    };
    let required_nodes = if let Some(val) = col_map.get("required_nodes") {
        extract_string_array(val)?
    } else {
        Vec::new()
    };

    let properties = if let Some(val) = col_map.get("properties") {
        Some(convert_property_value(val, "properties")?)
    } else {
        None
    };
    let initial_structure = if let Some(val) = col_map.get("initial_structure") {
        Some(convert_property_value(val, "initial_structure")?)
    } else {
        None
    };
    let overrides = if let Some(val) = col_map.get("overrides") {
        Some(convert_property_value(val, "overrides")?)
    } else {
        None
    };
    let index_types = if let Some(val) = col_map.get("index_types") {
        Some(convert_property_value(val, "index_types")?)
    } else {
        None
    };

    Ok(NodeType {
        id,
        strict,
        name,
        extends: extract_optional_string_column(col_map, "extends"),
        mixins,
        overrides,
        description,
        icon: extract_optional_string_column(col_map, "icon"),
        version,
        properties,
        allowed_children,
        required_nodes,
        initial_structure,
        versionable,
        publishable,
        auditable,
        indexable,
        index_types,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    })
}

/// Build an Archetype from column->value map.
pub(super) fn build_archetype_from_columns(
    col_map: &IndexMap<String, PropertyValue>,
) -> Result<Archetype, Error> {
    let id = extract_string_column(col_map, "id")?;
    let name = extract_string_column(col_map, "name")?;
    let version = col_map.get("version").and_then(|v| match v {
        PropertyValue::Integer(n) => Some(*n as i32),
        PropertyValue::Float(n) => Some(*n as i32),
        _ => None,
    });

    let description = extract_optional_string_column(col_map, "description");
    let publishable = extract_optional_boolean_column(col_map, "publishable");

    let fields = if let Some(val) = col_map.get("fields") {
        Some(convert_property_value(val, "fields")?)
    } else {
        None
    };
    let initial_content = if let Some(val) = col_map.get("initial_content") {
        Some(convert_property_value(val, "initial_content")?)
    } else {
        None
    };
    let layout = if let Some(val) = col_map.get("layout") {
        Some(convert_property_value(val, "layout")?)
    } else {
        None
    };
    let meta = if let Some(val) = col_map.get("meta") {
        Some(convert_property_value(val, "meta")?)
    } else {
        None
    };

    Ok(Archetype {
        id,
        name,
        extends: extract_optional_string_column(col_map, "extends"),
        strict: extract_optional_boolean_column(col_map, "strict"),
        icon: extract_optional_string_column(col_map, "icon"),
        title: extract_optional_string_column(col_map, "title"),
        description,
        base_node_type: extract_optional_string_column(col_map, "base_node_type"),
        fields,
        initial_content,
        layout,
        meta,
        version,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        publishable,
        previous_version: None,
    })
}

/// Build an ElementType from column->value map.
pub(super) fn build_elementtype_from_columns(
    col_map: &IndexMap<String, PropertyValue>,
) -> Result<ElementType, Error> {
    let id = extract_string_column(col_map, "id")?;
    let name = extract_string_column(col_map, "name")?;
    let version = col_map.get("version").and_then(|v| match v {
        PropertyValue::Integer(n) => Some(*n as i32),
        PropertyValue::Float(n) => Some(*n as i32),
        _ => None,
    });

    let description = extract_optional_string_column(col_map, "description");
    let publishable = extract_optional_boolean_column(col_map, "publishable");

    let fields = if let Some(val) = col_map.get("fields") {
        convert_property_value(val, "fields")?
    } else {
        Vec::new()
    };
    let initial_content = if let Some(val) = col_map.get("initial_content") {
        Some(convert_property_value(val, "initial_content")?)
    } else {
        None
    };
    let layout = if let Some(val) = col_map.get("layout") {
        Some(convert_property_value(val, "layout")?)
    } else {
        None
    };
    let meta = if let Some(val) = col_map.get("meta") {
        Some(convert_property_value(val, "meta")?)
    } else {
        None
    };

    Ok(ElementType {
        id,
        name,
        extends: extract_optional_string_column(col_map, "extends"),
        strict: extract_optional_boolean_column(col_map, "strict"),
        title: extract_optional_string_column(col_map, "title"),
        icon: extract_optional_string_column(col_map, "icon"),
        description,
        fields,
        initial_content,
        layout,
        meta,
        version,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        publishable,
        previous_version: None,
    })
}

/// Apply an assignment to a NodeType field.
pub(super) fn apply_assignment_to_nodetype(
    node_type: &mut NodeType,
    col_name: &str,
    value: PropertyValue,
) -> Result<(), Error> {
    match col_name {
        "name" => node_type.name = extract_string_value(&value)?,
        "description" => node_type.description = Some(extract_string_value(&value)?),
        "icon" => node_type.icon = Some(extract_string_value(&value)?),
        "extends" => node_type.extends = Some(extract_string_value(&value)?),
        "id" => node_type.id = Some(extract_string_value(&value)?),
        "version" => node_type.version = Some(extract_number_value(&value)? as i32),
        "strict" => node_type.strict = Some(extract_boolean_value(&value)?),
        "versionable" => node_type.versionable = Some(extract_boolean_value(&value)?),
        "publishable" => node_type.publishable = Some(extract_boolean_value(&value)?),
        "auditable" => node_type.auditable = Some(extract_boolean_value(&value)?),
        "indexable" => node_type.indexable = Some(extract_boolean_value(&value)?),
        "mixins" => node_type.mixins = extract_string_array(&value)?,
        "allowed_children" => node_type.allowed_children = extract_string_array(&value)?,
        "required_nodes" => node_type.required_nodes = extract_string_array(&value)?,
        "properties" => node_type.properties = Some(convert_property_value(&value, col_name)?),
        "initial_structure" => {
            node_type.initial_structure = Some(convert_property_value(&value, col_name)?)
        }
        "overrides" => node_type.overrides = Some(convert_property_value(&value, col_name)?),
        "index_types" => node_type.index_types = Some(convert_property_value(&value, col_name)?),
        _ => {
            return Err(Error::Validation(format!(
                "Cannot update column '{}' on NodeType",
                col_name
            )))
        }
    }
    Ok(())
}

/// Apply an assignment to an Archetype field.
pub(super) fn apply_assignment_to_archetype(
    archetype: &mut Archetype,
    col_name: &str,
    value: PropertyValue,
) -> Result<(), Error> {
    match col_name {
        "name" => archetype.name = extract_string_value(&value)?,
        "id" => archetype.id = extract_string_value(&value)?,
        "description" => archetype.description = Some(extract_string_value(&value)?),
        "icon" => archetype.icon = Some(extract_string_value(&value)?),
        "title" => archetype.title = Some(extract_string_value(&value)?),
        "extends" => archetype.extends = Some(extract_string_value(&value)?),
        "base_node_type" => archetype.base_node_type = Some(extract_string_value(&value)?),
        "version" => archetype.version = Some(extract_number_value(&value)? as i32),
        "publishable" => archetype.publishable = Some(extract_boolean_value(&value)?),
        "fields" => archetype.fields = Some(convert_property_value(&value, col_name)?),
        "initial_content" => {
            archetype.initial_content = Some(convert_property_value(&value, col_name)?)
        }
        "layout" => archetype.layout = Some(convert_property_value(&value, col_name)?),
        "meta" => archetype.meta = Some(convert_property_value(&value, col_name)?),
        _ => {
            return Err(Error::Validation(format!(
                "Cannot update column '{}' on Archetype",
                col_name
            )))
        }
    }
    Ok(())
}

/// Apply an assignment to an ElementType field.
pub(super) fn apply_assignment_to_elementtype(
    element_type: &mut ElementType,
    col_name: &str,
    value: PropertyValue,
) -> Result<(), Error> {
    match col_name {
        "name" => element_type.name = extract_string_value(&value)?,
        "id" => element_type.id = extract_string_value(&value)?,
        "description" => element_type.description = Some(extract_string_value(&value)?),
        "icon" => element_type.icon = Some(extract_string_value(&value)?),
        "version" => element_type.version = Some(extract_number_value(&value)? as i32),
        "publishable" => element_type.publishable = Some(extract_boolean_value(&value)?),
        "title" => element_type.title = Some(extract_string_value(&value)?),
        "fields" => element_type.fields = convert_property_value(&value, col_name)?,
        "initial_content" => {
            element_type.initial_content = Some(convert_property_value(&value, col_name)?)
        }
        "layout" => element_type.layout = Some(convert_property_value(&value, col_name)?),
        "meta" => element_type.meta = Some(convert_property_value(&value, col_name)?),
        _ => {
            return Err(Error::Validation(format!(
                "Cannot update column '{}' on ElementType",
                col_name
            )))
        }
    }
    Ok(())
}
