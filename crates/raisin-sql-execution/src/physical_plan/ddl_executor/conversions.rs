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
                // Array *items* cannot carry spatial config: the index is keyed
                // by property name, and an item has none. Area C's
                // `SPATIAL_INDEX(...)` property modifier fills this at the
                // top-level property instead.
                spatial: None,
                // Likewise for secrets: a secret is vaulted under a name derived
                // from the property path, which an anonymous item does not have.
                encrypted: None,
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
        // TODO(area-C): populate from the `SPATIAL_INDEX (<precisions>)` property
        // modifier once the DDL parser produces it. `None` inherits the workspace
        // defaults, so leaving it unset is correct behaviour meanwhile — geometry
        // properties are still indexed automatically by value type.
        spatial: None,
        // TODO(secrets): populate from an `ENCRYPTED` property modifier once the
        // DDL parser produces one. Until then a nodetype created through SQL DDL
        // cannot declare a secret field — declare it in YAML instead. `None` is
        // the safe default: not secret, stored as written.
        encrypted: None,
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

// =============================================================================
// FIELDS: PropertyDef -> FieldSchema
// =============================================================================
//
// ElementTypes and Archetypes declare FIELDS, not PROPERTIES, and a field is a
// tagged enum (`FieldSchema`) rather than the flat `PropertyValueSchema` a
// NodeType property uses. Both DDL executors used to answer "FieldSchema is
// complex" and store `fields: None` / `Vec::new()` — the statement parsed, the
// entity was created, the server said "created", and every field the author
// wrote was thrown away. One conversion lives here so `CREATE`, `ALTER ... ADD
// FIELD` and `ALTER ... MODIFY FIELD` cannot disagree about what a field means.

/// Convert DDL field definitions into element/archetype `FieldSchema` values.
pub(crate) fn convert_fields(
    fields: &[PropertyDef],
) -> Result<Vec<raisin_models::nodes::types::element::field_types::FieldSchema>, Error> {
    fields.iter().map(convert_field).collect()
}

/// Convert one DDL field definition into a `FieldSchema`.
///
/// The DDL type vocabulary is the property one, so the mapping picks the field
/// variant that carries the same value. `Array` has no field variant of its
/// own: a repeated field is the ELEMENT type with `multiple: true`, which is how
/// the YAML authoring format spells it too.
pub(crate) fn convert_field(
    prop: &PropertyDef,
) -> Result<raisin_models::nodes::types::element::field_types::FieldSchema, Error> {
    use raisin_models::nodes::types::element::field_types::FieldSchema;

    let (type_def, multiple) = match &prop.property_type {
        PropertyTypeDef::Array { items } => (items.as_ref(), Some(true)),
        other => (other, None),
    };

    let base = raisin_models::nodes::types::element::fields::base_field::FieldTypeSchema {
        name: prop.name.clone(),
        title: None,
        label: prop.label.clone(),
        required: if prop.required { Some(true) } else { None },
        description: prop.description.clone(),
        help_text: None,
        default_value: prop.default.as_ref().and_then(convert_default_value),
        validations: None,
        is_hidden: None,
        multiple,
        design_value: None,
        translatable: if prop.translatable { Some(true) } else { None },
        index: if prop.index.is_empty() {
            None
        } else {
            Some(convert_index_types(&prop.index))
        },
        meta: None,
        // No `ENCRYPTED` modifier exists in the DDL grammar yet, and `None` is
        // the safe default: not secret, stored as written. A secret field must
        // still be declared in YAML.
        encrypted: None,
    };

    Ok(match type_def {
        PropertyTypeDef::String | PropertyTypeDef::URL => {
            FieldSchema::TextField { base, config: None }
        }
        PropertyTypeDef::Number => FieldSchema::NumberField { base, config: None },
        PropertyTypeDef::Boolean => FieldSchema::BooleanField { base },
        PropertyTypeDef::Date => FieldSchema::DateField { base, config: None },
        PropertyTypeDef::Reference | PropertyTypeDef::NodeType => {
            FieldSchema::ReferenceField { base, config: None }
        }
        PropertyTypeDef::Resource => FieldSchema::MediaField { base, config: None },
        PropertyTypeDef::Object { .. } => FieldSchema::JsonObjectField { base },
        // An `Element` / `Composite` field holds element instances. The DDL type
        // vocabulary carries no element-type NAME, and `allowed_element_types:
        // None` is exactly "any element type" — the honest reading of a field
        // declared without one. A `Composite` is the repeated form of the same
        // thing. Erroring here instead would REJECT `FIELDS (hero Element)`,
        // which is accepted today (and is in the published DDL reference), so
        // the strictness would cost more than it bought.
        PropertyTypeDef::Element => FieldSchema::SectionField {
            base,
            allowed_element_types: None,
            render_as: None,
        },
        PropertyTypeDef::Composite => FieldSchema::SectionField {
            base: raisin_models::nodes::types::element::fields::base_field::FieldTypeSchema {
                multiple: Some(true),
                ..base
            },
            allowed_element_types: None,
            render_as: None,
        },
        // Unreachable through the parser: Array was unwrapped above and it does
        // not nest. A field is multiple or it is not.
        PropertyTypeDef::Array { .. } => {
            return Err(Error::Validation(format!(
                "field '{}': nested arrays are not supported; a repeated field is \
                 declared as `Array<T>` of a scalar type",
                prop.name
            )))
        }
    })
}

#[cfg(test)]
mod field_conversion_tests {
    use super::*;
    use raisin_models::nodes::types::element::field_types::{FieldSchema, FieldSchemaBase};

    fn field(name: &str, ty: PropertyTypeDef) -> PropertyDef {
        PropertyDef {
            name: name.to_string(),
            property_type: ty,
            required: true,
            ..Default::default()
        }
    }

    /// The DDL executors used to answer "FieldSchema is complex" and store an
    /// EMPTY field list. `CREATE ELEMENTTYPE 'x' FIELDS (...)` reported success
    /// and persisted nothing, so the entity existed with no schema and the
    /// editor rendered no form for it.
    #[test]
    fn ddl_field_definitions_convert_to_the_matching_field_variants() {
        let fields = convert_fields(&[
            field("headline", PropertyTypeDef::String),
            field("count", PropertyTypeDef::Number),
            field("live", PropertyTypeDef::Boolean),
            field("published", PropertyTypeDef::Date),
            field("author", PropertyTypeDef::Reference),
            field("payload", PropertyTypeDef::Object { fields: vec![] }),
        ])
        .expect("every scalar DDL type must convert");

        assert_eq!(fields.len(), 6);
        assert_eq!(fields[0].base_name(), "headline");
        assert!(matches!(fields[0], FieldSchema::TextField { .. }));
        assert!(matches!(fields[1], FieldSchema::NumberField { .. }));
        assert!(matches!(fields[2], FieldSchema::BooleanField { .. }));
        assert!(matches!(fields[3], FieldSchema::DateField { .. }));
        assert!(matches!(fields[4], FieldSchema::ReferenceField { .. }));
        assert!(matches!(fields[5], FieldSchema::JsonObjectField { .. }));
    }

    /// `REQUIRED` and the other modifiers must survive the conversion, or the
    /// schema stored is not the schema written.
    #[test]
    fn field_modifiers_reach_the_stored_schema() {
        let mut def = field("headline", PropertyTypeDef::String);
        def.label = Some("Headline".to_string());
        def.description = Some("The big text".to_string());
        def.translatable = true;

        let converted = convert_field(&def).unwrap();
        let FieldSchema::TextField { base, .. } = converted else {
            panic!("String must become a TextField");
        };
        assert_eq!(base.name, "headline");
        assert_eq!(base.required, Some(true));
        assert_eq!(base.label.as_deref(), Some("Headline"));
        assert_eq!(base.description.as_deref(), Some("The big text"));
        assert_eq!(base.translatable, Some(true));
        // A scalar is not multiple; only `Array<T>` sets that.
        assert_eq!(base.multiple, None);
    }

    /// `Array<T>` is the SAME field with `multiple: true` — the spelling the
    /// YAML authoring format uses — not a distinct array field variant.
    #[test]
    fn an_array_field_is_its_element_type_marked_multiple() {
        let def = field(
            "tags",
            PropertyTypeDef::Array {
                items: Box::new(PropertyTypeDef::String),
            },
        );
        let FieldSchema::TextField { base, .. } = convert_field(&def).unwrap() else {
            panic!("Array<String> must become a multiple TextField");
        };
        assert_eq!(base.multiple, Some(true));
        assert_eq!(base.name, "tags");
    }

    /// `Element` and `Composite` are accepted, not rejected: `FIELDS (hero
    /// Element)` is in the published DDL reference and writes successfully
    /// today. With no element-type name in the grammar, `allowed_element_types:
    /// None` — any element type — is the honest reading.
    #[test]
    fn element_and_composite_fields_become_open_section_fields() {
        let FieldSchema::SectionField {
            allowed_element_types,
            base,
            ..
        } = convert_field(&field("hero", PropertyTypeDef::Element)).unwrap()
        else {
            panic!("Element must become a SectionField");
        };
        assert!(allowed_element_types.is_none());
        assert_eq!(base.multiple, None);

        let FieldSchema::SectionField { base, .. } =
            convert_field(&field("body", PropertyTypeDef::Composite)).unwrap()
        else {
            panic!("Composite must become a SectionField");
        };
        assert_eq!(
            base.multiple,
            Some(true),
            "a Composite is the repeated form of an Element field"
        );
    }
}
