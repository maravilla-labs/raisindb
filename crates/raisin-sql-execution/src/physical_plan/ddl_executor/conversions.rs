//! Type conversion helpers for DDL operations.
//!
//! Converts DDL AST types (PropertyDef, PropertyTypeDef, etc.) into
//! the corresponding model types (PropertyValueSchema, PropertyType, etc.).

use raisin_error::Error;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition, IndexType, PropertyType,
    PropertyValueSchema,
};
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::ast::ddl::{
    CompoundIndexDef, DefaultValue, IndexTypeDef, PropertyDef, PropertyTypeDef,
};

/// Convert DDL property definitions to PropertyValueSchema
pub(crate) fn convert_properties(props: &[PropertyDef]) -> Result<Vec<PropertyValueSchema>, Error> {
    props.iter().map(convert_property).collect()
}

/// Convert a single DDL property definition to PropertyValueSchema
pub(crate) fn convert_property(prop: &PropertyDef) -> Result<PropertyValueSchema, Error> {
    use std::collections::HashMap;

    let meta = {
        let mut m = HashMap::new();
        if let Some(ref label) = prop.label {
            m.insert("label".to_string(), PropertyValue::String(label.clone()));
        }
        if let Some(ref desc) = prop.description {
            m.insert(
                "description".to_string(),
                PropertyValue::String(desc.clone()),
            );
        }
        if let Some(order) = prop.order {
            m.insert("order".to_string(), PropertyValue::Integer(order as i64));
        }
        if m.is_empty() {
            None
        } else {
            Some(m)
        }
    };

    let (structure, items) = match &prop.property_type {
        PropertyTypeDef::Object { fields } => {
            let mut struct_map = std::collections::HashMap::new();
            for field in fields {
                let field_schema = convert_property(field)?;
                struct_map.insert(field.name.clone(), field_schema);
            }
            (Some(struct_map), None)
        }
        PropertyTypeDef::Array { items } => {
            let item_schema = PropertyValueSchema {
                name: None,
                property_type: convert_property_type(items)?,
                required: None,
                unique: None,
                index: None,
                default: None,
                is_translatable: None,
                constraints: None,
                structure: None,
                items: None,
                value: None,
                meta: None,
                allow_additional_properties: None,
            };
            (None, Some(Box::new(item_schema)))
        }
        _ => (None, None),
    };

    Ok(PropertyValueSchema {
        name: Some(prop.name.clone()),
        property_type: convert_property_type(&prop.property_type)?,
        required: if prop.required { Some(true) } else { None },
        unique: if prop.unique { Some(true) } else { None },
        index: if prop.index.is_empty() {
            None
        } else {
            Some(convert_index_types(&prop.index))
        },
        default: prop.default.as_ref().and_then(convert_default_value),
        is_translatable: if prop.translatable { Some(true) } else { None },
        constraints: convert_constraints(&prop.constraints),
        structure,
        items,
        value: None,
        meta,
        allow_additional_properties: if prop.allow_additional_properties {
            Some(true)
        } else {
            None
        },
    })
}

/// Convert DDL property type to model PropertyType
fn convert_property_type(prop_type: &PropertyTypeDef) -> Result<PropertyType, Error> {
    match prop_type {
        PropertyTypeDef::String => Ok(PropertyType::String),
        PropertyTypeDef::Number => Ok(PropertyType::Number),
        PropertyTypeDef::Boolean => Ok(PropertyType::Boolean),
        PropertyTypeDef::Date => Ok(PropertyType::Date),
        PropertyTypeDef::URL => Ok(PropertyType::URL),
        PropertyTypeDef::Reference => Ok(PropertyType::Reference),
        PropertyTypeDef::Resource => Ok(PropertyType::Resource),
        PropertyTypeDef::Composite => Ok(PropertyType::Composite),
        PropertyTypeDef::Element => Ok(PropertyType::Element),
        PropertyTypeDef::NodeType => Ok(PropertyType::NodeType),
        PropertyTypeDef::Object { .. } => Ok(PropertyType::Object),
        PropertyTypeDef::Array { .. } => Ok(PropertyType::Array),
    }
}

/// Convert DDL index types to model IndexTypes
fn convert_index_types(indexes: &[IndexTypeDef]) -> Vec<IndexType> {
    indexes
        .iter()
        .map(|idx| match idx {
            IndexTypeDef::Fulltext => IndexType::Fulltext,
            IndexTypeDef::Vector => IndexType::Vector,
            IndexTypeDef::Property => IndexType::Property,
        })
        .collect()
}

/// Convert DDL compound index definitions to model CompoundIndexDefinition
pub(crate) fn convert_compound_indexes(
    indexes: &[CompoundIndexDef],
) -> Vec<CompoundIndexDefinition> {
    indexes
        .iter()
        .map(|idx| CompoundIndexDefinition {
            name: idx.name.clone(),
            columns: idx
                .columns
                .iter()
                .map(|col| CompoundIndexColumn {
                    property: col.property.clone(),
                    ascending: Some(col.ascending),
                    column_type: infer_column_type(&col.property),
                })
                .collect(),
            has_order_column: idx.has_order_column,
        })
        .collect()
}

/// Infer the column type from the property name for proper key encoding
fn infer_column_type(property: &str) -> CompoundColumnType {
    match property {
        "__created_at" | "__updated_at" => CompoundColumnType::Timestamp,
        _ => CompoundColumnType::String,
    }
}

/// Convert DDL default value to PropertyValue
fn convert_default_value(default: &DefaultValue) -> Option<PropertyValue> {
    match default {
        DefaultValue::String(s) => Some(PropertyValue::String(s.clone())),
        DefaultValue::Number(n) => Some(PropertyValue::Float(*n)),
        DefaultValue::Boolean(b) => Some(PropertyValue::Boolean(*b)),
        DefaultValue::Null => Some(PropertyValue::Null),
    }
}

/// Convert JSON constraints to HashMap<String, PropertyValue>
///
/// Note: This is a simplified conversion - complex nested structures may not
/// convert perfectly.
fn convert_constraints(
    constraints: &Option<serde_json::Value>,
) -> Option<std::collections::HashMap<String, PropertyValue>> {
    use std::collections::HashMap;

    constraints.as_ref().and_then(|value| {
        if let serde_json::Value::Object(obj) = value {
            let mut map = HashMap::new();
            for (key, val) in obj {
                if let Some(pv) = json_to_property_value(val) {
                    map.insert(key.clone(), pv);
                }
            }
            if map.is_empty() {
                None
            } else {
                Some(map)
            }
        } else {
            None
        }
    })
}

/// Convert a serde_json::Value to PropertyValue
fn json_to_property_value(value: &serde_json::Value) -> Option<PropertyValue> {
    match value {
        serde_json::Value::String(s) => Some(PropertyValue::String(s.clone())),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(PropertyValue::Integer(i))
            } else {
                n.as_f64().map(PropertyValue::Float)
            }
        }
        serde_json::Value::Bool(b) => Some(PropertyValue::Boolean(*b)),
        serde_json::Value::Null => Some(PropertyValue::Null),
        serde_json::Value::Array(arr) => {
            let items: Vec<PropertyValue> = arr.iter().filter_map(json_to_property_value).collect();
            Some(PropertyValue::Array(items))
        }
        serde_json::Value::Object(obj) => {
            let mut map = std::collections::HashMap::new();
            for (k, v) in obj {
                if let Some(pv) = json_to_property_value(v) {
                    map.insert(k.clone(), pv);
                }
            }
            Some(PropertyValue::Object(map))
        }
    }
}
