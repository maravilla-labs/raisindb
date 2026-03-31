# Property Schemas

Complete guide to defining property schemas in RaisinDB NodeTypes.

## Overview

Properties define the fields that nodes will have. Each property is defined using a `PropertyValueSchema` which specifies the type, validation rules, and behavior.

```rust
use raisin_models::nodes::properties::schema::PropertyValueSchema;

let title_property = PropertyValueSchema {
    name: Some("title".to_string()),
    property_type: PropertyType::String,
    required: Some(true),
    unique: Some(false),
    default: None,
    constraints: None,
    is_translatable: Some(true),
    ..Default::default()
};
```

## PropertyValueSchema Structure

```rust
pub struct PropertyValueSchema {
    pub name: Option<String>,                    // Property name
    pub property_type: PropertyType,             // Data type
    pub required: Option<bool>,                  // Must be present
    pub unique: Option<bool>,                    // Must be unique across nodes
    pub default: Option<PropertyValue>,          // Default value
    pub constraints: Option<HashMap<String, PropertyValue>>,  // Validation rules
    pub structure: Option<HashMap<String, PropertyValueSchema>>,  // For Object type
    pub items: Option<Box<PropertyValueSchema>>, // For Array type
    pub value: Option<PropertyValue>,            // Fixed value
    pub meta: Option<HashMap<String, PropertyValue>>,  // Additional metadata
    pub is_translatable: Option<bool>,           // Enable translations
    pub allow_additional_properties: Option<bool>,  // For Object type
    pub index: Option<Vec<IndexType>>,           // Index types (Fulltext, Vector, Property)
}
```

## Basic Properties

### Required Properties

Mark a property as required:

```rust
PropertyValueSchema {
    name: Some("email".to_string()),
    property_type: PropertyType::String,
    required: Some(true),  // Must be provided
    ..Default::default()
}
```

Attempting to create a node without a required property will fail:

```rust
// ❌ Error: Missing required property 'email'
let node = Node {
    node_type: "User".to_string(),
    properties: HashMap::new(),  // Missing required 'email'
    ..Default::default()
};
```

### Optional Properties

Properties are optional by default:

```rust
PropertyValueSchema {
    name: Some("bio".to_string()),
    property_type: PropertyType::String,
    required: Some(false),  // Or omit - defaults to false
    ..Default::default()
}
```

### Unique Properties

Ensure property values are unique across all nodes of this type:

```rust
PropertyValueSchema {
    name: Some("username".to_string()),
    property_type: PropertyType::String,
    required: Some(true),
    unique: Some(true),  // No two nodes can have same username
    ..Default::default()
}
```

## Default Values

Provide default values for properties:

```rust
PropertyValueSchema {
    name: Some("status".to_string()),
    property_type: PropertyType::String,
    default: Some(PropertyValue::String("draft".to_string())),
    ..Default::default()
}
```

When creating a node, if this property isn't provided, it uses the default:

```rust
let node = Node {
    node_type: "Article".to_string(),
    properties: HashMap::new(),  // status will be "draft"
    ..Default::default()
};
```

## Constraints

Add validation rules using the `constraints` field:

### String Constraints

```rust
use std::collections::HashMap;

let mut constraints = HashMap::new();
constraints.insert("minLength".to_string(), PropertyValue::Number(3.0));
constraints.insert("maxLength".to_string(), PropertyValue::Number(50.0));
constraints.insert("pattern".to_string(),
    PropertyValue::String("^[a-zA-Z0-9_]+$".to_string()));

PropertyValueSchema {
    name: Some("username".to_string()),
    property_type: PropertyType::String,
    required: Some(true),
    constraints: Some(constraints),
    ..Default::default()
}
```

Common string constraints:
- `minLength` - Minimum string length
- `maxLength` - Maximum string length
- `pattern` - Regular expression pattern

### Number Constraints

```rust
let mut constraints = HashMap::new();
constraints.insert("minimum".to_string(), PropertyValue::Number(0.0));
constraints.insert("maximum".to_string(), PropertyValue::Number(100.0));
constraints.insert("multipleOf".to_string(), PropertyValue::Number(5.0));

PropertyValueSchema {
    name: Some("score".to_string()),
    property_type: PropertyType::Number,
    constraints: Some(constraints),
    ..Default::default()
}
```

Common number constraints:
- `minimum` - Minimum value
- `maximum` - Maximum value
- `multipleOf` - Must be multiple of this value
- `exclusiveMinimum` - Minimum value (exclusive)
- `exclusiveMaximum` - Maximum value (exclusive)

### Array Constraints

```rust
let mut constraints = HashMap::new();
constraints.insert("minItems".to_string(), PropertyValue::Number(1.0));
constraints.insert("maxItems".to_string(), PropertyValue::Number(10.0));
constraints.insert("uniqueItems".to_string(), PropertyValue::Boolean(true));

PropertyValueSchema {
    name: Some("tags".to_string()),
    property_type: PropertyType::Array,
    constraints: Some(constraints),
    ..Default::default()
}
```

Common array constraints:
- `minItems` - Minimum array length
- `maxItems` - Maximum array length
- `uniqueItems` - All items must be unique

## Complex Property Types

### Object Properties

Define nested object structures:

```rust
let mut address_structure = HashMap::new();

address_structure.insert("street".to_string(), PropertyValueSchema {
    name: Some("street".to_string()),
    property_type: PropertyType::String,
    required: Some(true),
    ..Default::default()
});

address_structure.insert("city".to_string(), PropertyValueSchema {
    name: Some("city".to_string()),
    property_type: PropertyType::String,
    required: Some(true),
    ..Default::default()
});

address_structure.insert("zipCode".to_string(), PropertyValueSchema {
    name: Some("zipCode".to_string()),
    property_type: PropertyType::String,
    constraints: Some({
        let mut c = HashMap::new();
        c.insert("pattern".to_string(),
            PropertyValue::String("^[0-9]{5}$".to_string()));
        c
    }),
    ..Default::default()
});

PropertyValueSchema {
    name: Some("address".to_string()),
    property_type: PropertyType::Object,
    structure: Some(address_structure),  // Define nested schema
    allow_additional_properties: Some(false),  // Only allow defined properties
    ..Default::default()
}
```

Usage:

```rust
let mut address = HashMap::new();
address.insert("street".to_string(), PropertyValue::String("123 Main St".to_string()));
address.insert("city".to_string(), PropertyValue::String("Springfield".to_string()));
address.insert("zipCode".to_string(), PropertyValue::String("12345".to_string()));

node.properties.insert("address".to_string(), PropertyValue::Object(address));
```

### Array Properties

Define arrays with typed items:

```rust
PropertyValueSchema {
    name: Some("tags".to_string()),
    property_type: PropertyType::Array,
    items: Some(Box::new(PropertyValueSchema {
        property_type: PropertyType::String,  // Array of strings
        constraints: Some({
            let mut c = HashMap::new();
            c.insert("minLength".to_string(), PropertyValue::Number(2.0));
            c
        }),
        ..Default::default()
    })),
    ..Default::default()
}
```

Array of objects:

```rust
PropertyValueSchema {
    name: Some("authors".to_string()),
    property_type: PropertyType::Array,
    items: Some(Box::new(PropertyValueSchema {
        property_type: PropertyType::Object,
        structure: Some({
            let mut structure = HashMap::new();
            structure.insert("name".to_string(), PropertyValueSchema {
                property_type: PropertyType::String,
                required: Some(true),
                ..Default::default()
            });
            structure.insert("email".to_string(), PropertyValueSchema {
                property_type: PropertyType::String,
                ..Default::default()
            });
            structure
        }),
        ..Default::default()
    })),
    ..Default::default()
}
```

## Translatable Properties

Enable multi-language support for properties:

```rust
PropertyValueSchema {
    name: Some("title".to_string()),
    property_type: PropertyType::String,
    is_translatable: Some(true),  // Can be translated
    ..Default::default()
}
```

When a property is translatable, nodes can store translations:

```rust
// Primary value
node.properties.insert("title".to_string(),
    PropertyValue::String("Hello World".to_string()));

// Translations
let mut translations = HashMap::new();
translations.insert("title.es".to_string(),
    PropertyValue::String("Hola Mundo".to_string()));
translations.insert("title.fr".to_string(),
    PropertyValue::String("Bonjour le monde".to_string()));

node.translations = Some(translations);
```

## Meta Field

Store additional metadata about the property:

```rust
PropertyValueSchema {
    name: Some("rating".to_string()),
    property_type: PropertyType::Number,
    meta: Some({
        let mut m = HashMap::new();
        m.insert("displayName".to_string(),
            PropertyValue::String("User Rating".to_string()));
        m.insert("description".to_string(),
            PropertyValue::String("Rating from 1 to 5 stars".to_string()));
        m.insert("uiWidget".to_string(),
            PropertyValue::String("stars".to_string()));
        m.insert("helpText".to_string(),
            PropertyValue::String("Select a rating between 1 and 5".to_string()));
        m
    }),
    ..Default::default()
}
```

The `meta` field is a `HashMap<String, PropertyValue>` - useful for:
- UI hints (widget type, display name)
- Documentation (help text, examples)
- Custom validation logic
- Integration metadata

## Complete Examples

### User Profile

```rust
let user_type = NodeType {
    name: "User".to_string(),
    properties: Some(vec![
        // Required unique username
        PropertyValueSchema {
            name: Some("username".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
            unique: Some(true),
            constraints: Some({
                let mut c = HashMap::new();
                c.insert("minLength".to_string(), PropertyValue::Number(3.0));
                c.insert("maxLength".to_string(), PropertyValue::Number(20.0));
                c.insert("pattern".to_string(),
                    PropertyValue::String("^[a-zA-Z0-9_]+$".to_string()));
                c
            }),
            ..Default::default()
        },
        // Required email
        PropertyValueSchema {
            name: Some("email".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
            unique: Some(true),
            constraints: Some({
                let mut c = HashMap::new();
                c.insert("pattern".to_string(),
                    PropertyValue::String(r"^[^\s@]+@[^\s@]+\.[^\s@]+$".to_string()));
                c
            }),
            ..Default::default()
        },
        // Optional bio (translatable)
        PropertyValueSchema {
            name: Some("bio".to_string()),
            property_type: PropertyType::String,
            is_translatable: Some(true),
            constraints: Some({
                let mut c = HashMap::new();
                c.insert("maxLength".to_string(), PropertyValue::Number(500.0));
                c
            }),
            ..Default::default()
        },
        // Status with default
        PropertyValueSchema {
            name: Some("status".to_string()),
            property_type: PropertyType::String,
            default: Some(PropertyValue::String("active".to_string())),
            constraints: Some({
                let mut c = HashMap::new();
                c.insert("enum".to_string(), PropertyValue::Array(vec![
                    PropertyValue::String("active".to_string()),
                    PropertyValue::String("inactive".to_string()),
                    PropertyValue::String("suspended".to_string()),
                ]));
                c
            }),
            ..Default::default()
        },
    ]),
    ..Default::default()
};
```

### Blog Post

```rust
let blog_post = NodeType {
    name: "BlogPost".to_string(),
    properties: Some(vec![
        // Translatable title
        PropertyValueSchema {
            name: Some("title".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
            is_translatable: Some(true),
            constraints: Some({
                let mut c = HashMap::new();
                c.insert("minLength".to_string(), PropertyValue::Number(1.0));
                c.insert("maxLength".to_string(), PropertyValue::Number(200.0));
                c
            }),
            ..Default::default()
        },
        // Slug (unique URL-friendly identifier)
        PropertyValueSchema {
            name: Some("slug".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
            unique: Some(true),
            constraints: Some({
                let mut c = HashMap::new();
                c.insert("pattern".to_string(),
                    PropertyValue::String("^[a-z0-9-]+$".to_string()));
                c
            }),
            ..Default::default()
        },
        // Translatable content
        PropertyValueSchema {
            name: Some("content".to_string()),
            property_type: PropertyType::String,
            is_translatable: Some(true),
            meta: Some({
                let mut m = HashMap::new();
                m.insert("uiWidget".to_string(),
                    PropertyValue::String("richtext".to_string()));
                m
            }),
            ..Default::default()
        },
        // Published date
        PropertyValueSchema {
            name: Some("publishedDate".to_string()),
            property_type: PropertyType::Date,
            ..Default::default()
        },
        // Tags array
        PropertyValueSchema {
            name: Some("tags".to_string()),
            property_type: PropertyType::Array,
            items: Some(Box::new(PropertyValueSchema {
                property_type: PropertyType::String,
                constraints: Some({
                    let mut c = HashMap::new();
                    c.insert("minLength".to_string(), PropertyValue::Number(2.0));
                    c
                }),
                ..Default::default()
            })),
            constraints: Some({
                let mut c = HashMap::new();
                c.insert("maxItems".to_string(), PropertyValue::Number(10.0));
                c
            }),
            ..Default::default()
        },
        // Author reference
        PropertyValueSchema {
            name: Some("author".to_string()),
            property_type: PropertyType::Reference,
            required: Some(true),
            ..Default::default()
        },
    ]),
    versionable: Some(true),
    publishable: Some(true),
    ..Default::default()
};
```

### Product Catalog

```rust
let product = NodeType {
    name: "Product".to_string(),
    properties: Some(vec![
        // Product name
        PropertyValueSchema {
            name: Some("name".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
            is_translatable: Some(true),
            ..Default::default()
        },
        // SKU (unique identifier)
        PropertyValueSchema {
            name: Some("sku".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
            unique: Some(true),
            ..Default::default()
        },
        // Price
        PropertyValueSchema {
            name: Some("price".to_string()),
            property_type: PropertyType::Number,
            required: Some(true),
            constraints: Some({
                let mut c = HashMap::new();
                c.insert("minimum".to_string(), PropertyValue::Number(0.0));
                c.insert("multipleOf".to_string(), PropertyValue::Number(0.01));
                c
            }),
            meta: Some({
                let mut m = HashMap::new();
                m.insert("currency".to_string(),
                    PropertyValue::String("USD".to_string()));
                m.insert("uiWidget".to_string(),
                    PropertyValue::String("currency".to_string()));
                m
            }),
            ..Default::default()
        },
        // Stock quantity
        PropertyValueSchema {
            name: Some("stock".to_string()),
            property_type: PropertyType::Number,
            default: Some(PropertyValue::Number(0.0)),
            constraints: Some({
                let mut c = HashMap::new();
                c.insert("minimum".to_string(), PropertyValue::Number(0.0));
                c.insert("multipleOf".to_string(), PropertyValue::Number(1.0));
                c
            }),
            ..Default::default()
        },
        // Dimensions (nested object)
        PropertyValueSchema {
            name: Some("dimensions".to_string()),
            property_type: PropertyType::Object,
            structure: Some({
                let mut structure = HashMap::new();
                structure.insert("width".to_string(), PropertyValueSchema {
                    property_type: PropertyType::Number,
                    constraints: Some({
                        let mut c = HashMap::new();
                        c.insert("minimum".to_string(), PropertyValue::Number(0.0));
                        c
                    }),
                    ..Default::default()
                });
                structure.insert("height".to_string(), PropertyValueSchema {
                    property_type: PropertyType::Number,
                    constraints: Some({
                        let mut c = HashMap::new();
                        c.insert("minimum".to_string(), PropertyValue::Number(0.0));
                        c
                    }),
                    ..Default::default()
                });
                structure.insert("depth".to_string(), PropertyValueSchema {
                    property_type: PropertyType::Number,
                    constraints: Some({
                        let mut c = HashMap::new();
                        c.insert("minimum".to_string(), PropertyValue::Number(0.0));
                        c
                    }),
                    ..Default::default()
                });
                structure.insert("unit".to_string(), PropertyValueSchema {
                    property_type: PropertyType::String,
                    default: Some(PropertyValue::String("cm".to_string())),
                    ..Default::default()
                });
                structure
            }),
            ..Default::default()
        },
        // Product images
        PropertyValueSchema {
            name: Some("images".to_string()),
            property_type: PropertyType::Array,
            items: Some(Box::new(PropertyValueSchema {
                property_type: PropertyType::Resource,
                ..Default::default()
            })),
            constraints: Some({
                let mut c = HashMap::new();
                c.insert("maxItems".to_string(), PropertyValue::Number(5.0));
                c
            }),
            ..Default::default()
        },
    ]),
    ..Default::default()
};
```

## Best Practices

### 1. Use Descriptive Names

```rust
// ✅ Good
"email", "firstName", "publishedDate"

// ❌ Avoid
"e", "fn", "pd"
```

### 2. Set Appropriate Constraints

```rust
// ✅ Good - prevents issues
PropertyValueSchema {
    name: Some("age".to_string()),
    property_type: PropertyType::Number,
    constraints: Some({
        let mut c = HashMap::new();
        c.insert("minimum".to_string(), PropertyValue::Number(0.0));
        c.insert("maximum".to_string(), PropertyValue::Number(150.0));
        c
    }),
    ..Default::default()
}
```

### 3. Use Defaults Wisely

```rust
// ✅ Good - sensible default
PropertyValueSchema {
    name: Some("status".to_string()),
    property_type: PropertyType::String,
    default: Some(PropertyValue::String("draft".to_string())),
    ..Default::default()
}
```

### 4. Mark Required Fields

```rust
// ✅ Good - explicit about requirements
PropertyValueSchema {
    name: Some("email".to_string()),
    property_type: PropertyType::String,
    required: Some(true),  // Clear expectation
    ..Default::default()
}
```

### 5. Enable Translations When Needed

```rust
// ✅ Good - user-facing content is translatable
PropertyValueSchema {
    name: Some("description".to_string()),
    property_type: PropertyType::String,
    is_translatable: Some(true),
    ..Default::default()
}
```

### 6. Use Meta for UI Hints

```rust
// Good - helps UI builders
PropertyValueSchema {
    name: Some("coverImage".to_string()),
    property_type: PropertyType::Resource,
    meta: Some({
        let mut m = HashMap::new();
        m.insert("accept".to_string(),
            PropertyValue::String("image/*".to_string()));
        m.insert("maxSize".to_string(),
            PropertyValue::Number(5242880.0));  // 5MB
        m.insert("aspectRatio".to_string(),
            PropertyValue::String("16:9".to_string()));
        m
    }),
    ..Default::default()
}
```

## Next Steps

- [Property Type Reference](property-reference.md) - Detailed reference for each property type
- [Node System](../architecture/node-system.md) - Understanding NodeTypes
- [Nodes and Instances](nodes-and-instances.md) - Working with node instances
