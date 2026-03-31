# Node System

Understanding RaisinDB's node type system, schemas, and inheritance.

## Overview

RaisinDB uses a **schema-based node system** where you define **NodeTypes** that act as blueprints for creating **Nodes**. This is similar to:
- Classes and instances in OOP
- Tables and rows in databases
- Content types and content in CMSs

```
NodeType (Blueprint) → Node (Instance)
    ↓                      ↓
"BlogPost" type       Actual blog post with content
```

## NodeType Structure

A `NodeType` defines the schema, validation rules, and behavior for nodes:

```rust
pub struct NodeType {
    pub id: Option<String>,              // Unique identifier (auto-generated)
    pub name: String,                    // Type name (e.g., "BlogPost")
    pub strict: Option<bool>,            // Strict schema validation
    pub extends: Option<String>,         // Inherit from another type
    pub mixins: Vec<String>,             // Compose from multiple types
    pub overrides: Option<OverrideProperties>,  // Override inherited properties
    pub description: Option<String>,     // Human-readable description
    pub icon: Option<String>,            // Icon identifier
    pub version: Option<i32>,            // Schema version number
    pub properties: Option<Vec<PropertyValueSchema>>,  // Field definitions
    pub allowed_children: Vec<String>,   // Which types can be children
    pub required_nodes: Vec<String>,     // Required child nodes
    pub initial_structure: Option<InitialNodeStructure>,  // Auto-created children
    pub versionable: Option<bool>,       // Enable versioning
    pub publishable: Option<bool>,       // Enable publish/draft
    pub auditable: Option<bool>,         // Enable audit log
    pub indexable: Option<bool>,         // Whether this type should be indexed
    pub index_types: Option<Vec<IndexType>>,  // Which index types are enabled
    pub compound_indexes: Option<Vec<CompoundIndexDefinition>>,  // Multi-column indexes
    pub is_mixin: Option<bool>,          // Whether this type is a mixin
    // ... timestamps, etc.
}
```

## Creating NodeTypes

### Basic NodeType

```rust
use raisin_models::nodes::types::NodeType;
use raisin_core::NodeTypeService;

let page_type = NodeType {
    name: "Page".to_string(),
    description: Some("A basic web page".to_string()),
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("title".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
            ..Default::default()
        },
        PropertyValueSchema {
            name: Some("content".to_string()),
            property_type: PropertyType::String,
            required: Some(false),
            ..Default::default()
        },
    ]),
    allowed_children: vec!["Page".to_string(), "Section".to_string()],
    versionable: Some(true),
    publishable: Some(true),
    ..Default::default()
};

// Store the NodeType
let node_type_service = NodeTypeService::new(storage);
node_type_service.put("workspace", page_type).await?;
```

### NodeType with Inheritance

Use `extends` to inherit from another NodeType:

```rust
// Base type
let base_content = NodeType {
    name: "BaseContent".to_string(),
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("title".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
            ..Default::default()
        },
        PropertyValueSchema {
            name: Some("author".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
            ..Default::default()
        },
    ]),
    versionable: Some(true),
    auditable: Some(true),
    ..Default::default()
};

// Derived type - inherits title and author
let blog_post = NodeType {
    name: "BlogPost".to_string(),
    extends: Some("BaseContent".to_string()),  // Inherit from BaseContent
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("excerpt".to_string()),
            property_type: PropertyType::String,
            ..Default::default()
        },
        PropertyValueSchema {
            name: Some("published_date".to_string()),
            property_type: PropertyType::Date,
            ..Default::default()
        },
    ]),
    publishable: Some(true),
    ..Default::default()
};

// BlogPost now has: title, author (from BaseContent) + excerpt, published_date
```

### NodeType with Mixins

Mixins are **first-class entities** in RaisinDB. A mixin is a `NodeType` with `is_mixin: Some(true)`, which marks it as a reusable property set rather than a standalone content type. Mixins have their own dedicated SQL syntax (`CREATE MIXIN`, `ALTER MIXIN`, `DROP MIXIN`) -- see the [SQL Reference](../api/sql-reference.md#createalterdrop-mixin) for details.

#### Creating Mixins via SQL

The preferred way to create mixins is through SQL:

```sql
CREATE MIXIN 'myapp:SEO'
  DESCRIPTION 'SEO metadata fields'
  ICON 'search'
  PROPERTIES (
    meta_title String,
    meta_description String
  );

CREATE MIXIN 'myapp:Timestamps'
  DESCRIPTION 'Standard timestamp fields'
  PROPERTIES (
    created_at Date REQUIRED,
    updated_at Date REQUIRED
  );

-- Use mixins in a node type
CREATE NODETYPE 'myapp:Article'
  MIXINS ('myapp:SEO', 'myapp:Timestamps')
  PROPERTIES (
    title String REQUIRED
  );
```

#### Creating Mixins via Rust API

You can also create mixins programmatically. Set `is_mixin: Some(true)` to distinguish a mixin from a regular NodeType:

```rust
// Mixin for SEO fields
let seo_mixin = NodeType {
    name: "myapp:SEO".to_string(),
    is_mixin: Some(true),  // Marks this as a mixin
    description: Some("SEO metadata fields".to_string()),
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("meta_title".to_string()),
            property_type: PropertyType::String,
            ..Default::default()
        },
        PropertyValueSchema {
            name: Some("meta_description".to_string()),
            property_type: PropertyType::String,
            ..Default::default()
        },
    ]),
    ..Default::default()
};

// Mixin for timestamps
let timestamp_mixin = NodeType {
    name: "myapp:Timestamps".to_string(),
    is_mixin: Some(true),  // Marks this as a mixin
    description: Some("Standard timestamp fields".to_string()),
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("created_at".to_string()),
            property_type: PropertyType::Date,
            ..Default::default()
        },
        PropertyValueSchema {
            name: Some("updated_at".to_string()),
            property_type: PropertyType::Date,
            ..Default::default()
        },
    ]),
    ..Default::default()
};

// Compose from multiple mixins
let article = NodeType {
    name: "Article".to_string(),
    mixins: vec![
        "myapp:SEO".to_string(),
        "myapp:Timestamps".to_string(),
    ],
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("title".to_string()),
            property_type: PropertyType::String,
            ..Default::default()
        },
    ]),
    ..Default::default()
};

// Article now has: title + meta_title, meta_description + created_at, updated_at
```

## Property Schemas

Properties define the fields that nodes of this type will have:

```rust
PropertyValueSchema {
    name: Some("email".to_string()),
    property_type: PropertyType::String,
    required: Some(true),
    constraints: Some(HashMap::from([
        ("pattern".to_string(), PropertyValue::String(r"^[^\s@]+@[^\s@]+\.[^\s@]+$".to_string())),
        ("min_length".to_string(), PropertyValue::Number(5.into())),
        ("max_length".to_string(), PropertyValue::Number(255.into())),
    ])),
    default: None,
    ..Default::default()
}
```

### Property Types

```rust
pub enum PropertyType {
    String,         // Text values
    Number,         // Numeric values
    Boolean,        // true/false
    Array,          // Array of values
    Object,         // Nested object
    Date,           // Timestamps
    URL,            // URL values
    Reference,      // Reference to another node (auto-indexed!)
    NodeType,       // Reference to a NodeType
    Element,        // Inline element
    Composite,      // Composite structured value
    Resource,       // External resource reference
}
```

**Note**: `Reference` properties are automatically indexed by RaisinDB, enabling fast bidirectional lookups. See the [Property Type Reference Guide](../guides/property-reference.md#reference-indexing) for details on querying references.

## Allowed Children

Control which NodeTypes can be children:

```rust
let folder = NodeType {
    name: "Folder".to_string(),
    allowed_children: vec![
        "Folder".to_string(),      // Folders can contain folders
        "Page".to_string(),        // Folders can contain pages
        "Image".to_string(),       // Folders can contain images
    ],
    ..Default::default()
};

let page = NodeType {
    name: "Page".to_string(),
    allowed_children: vec![
        "Section".to_string(),     // Pages can only contain sections
        "Widget".to_string(),
    ],
    ..Default::default()
};

let image = NodeType {
    name: "Image".to_string(),
    allowed_children: vec![],       // Images cannot have children
    ..Default::default()
};
```

When adding a node, RaisinDB validates that the parent allows this child type:

```rust
// ✅ OK: Folder allows Page children
service.add_node("workspace", "/my-folder", page_node).await?;

// ❌ Error: Image doesn't allow children
service.add_node("workspace", "/my-image", page_node).await?;  // Fails!
```

## Required Nodes

Specify required child nodes:

```rust
let website = NodeType {
    name: "Website".to_string(),
    required_nodes: vec![
        "header".to_string(),
        "footer".to_string(),
        "navigation".to_string(),
    ],
    ..Default::default()
};

// When creating a Website node, you MUST create these children
```

## Initial Structure

Automatically create child nodes when a node of this type is created:

```rust
let blog = NodeType {
    name: "Blog".to_string(),
    initial_structure: Some(InitialNodeStructure {
        properties: None,
        children: Some(vec![
            InitialChild {
                name: "posts".to_string(),
                node_type: "Folder".to_string(),
                properties: Some(hashmap!{
                    "title".to_string() => "Blog Posts".into(),
                }),
                children: None,
            },
            InitialChild {
                name: "categories".to_string(),
                node_type: "Folder".to_string(),
                properties: Some(hashmap!{
                    "title".to_string() => "Categories".into(),
                }),
                children: None,
            },
        ]),
    }),
    ..Default::default()
};

// When you create a Blog node, it automatically creates:
// /my-blog/posts
// /my-blog/categories
```

### Nested Initial Structure

```rust
let project = NodeType {
    name: "Project".to_string(),
    initial_structure: Some(InitialNodeStructure {
        children: Some(vec![
            InitialChild {
                name: "src".to_string(),
                node_type: "Folder".to_string(),
                children: Some(vec![  // Nested!
                    InitialChild {
                        name: "components".to_string(),
                        node_type: "Folder".to_string(),
                        children: None,
                    },
                    InitialChild {
                        name: "utils".to_string(),
                        node_type: "Folder".to_string(),
                        children: None,
                    },
                ]),
                properties: None,
                translations: None,
                content_type: None,
            },
        ]),
        properties: None,
    }),
    ..Default::default()
};

// Creates:
// /my-project/src/
// /my-project/src/components/
// /my-project/src/utils/
```

## NodeType Features

### Versionable

Enable version history for nodes of this type:

```rust
let document = NodeType {
    name: "Document".to_string(),
    versionable: Some(true),  // Track all changes
    ..Default::default()
};

// Now you can:
// - Get version history
// - Restore previous versions
// - Compare versions
```

### Publishable

Enable draft/published workflow:

```rust
let article = NodeType {
    name: "Article".to_string(),
    publishable: Some(true),  // Enable publish workflow
    versionable: Some(true),  // Usually combined with versioning
    ..Default::default()
};

// Now nodes have:
// - draft version
// - published version
// - publish/unpublish operations
```

### Auditable

Enable audit logging:

```rust
let sensitive_data = NodeType {
    name: "CustomerRecord".to_string(),
    auditable: Some(true),  // Log all changes
    ..Default::default()
};

// All operations logged:
// - Who created/updated/deleted
// - When
// - What changed
```

### Strict Mode

Enforce strict schema validation:

```rust
let strict_type = NodeType {
    name: "StrictData".to_string(),
    strict: Some(true),  // Only defined properties allowed
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("approved_field".to_string()),
            property_type: PropertyType::String,
            ..Default::default()
        },
    ]),
    ..Default::default()
};

// With strict mode:
node.properties.insert("approved_field", value);  // ✅ OK
node.properties.insert("random_field", value);    // ❌ Error!

// Without strict mode (default):
node.properties.insert("random_field", value);    // ✅ OK (allows extra fields)
```

## Global NodeTypes

RaisinDB provides built-in global types:

```rust
// Available out of the box:
"raisin:Folder"     // Container for other nodes
"raisin:Page"       // Basic page with title/content
"raisin:Asset"      // Media asset (image, video, etc.)
"raisin:Link"       // URL link
```

Use global types without creating them:

```rust
let node = Node {
    name: "my-folder".to_string(),
    node_type: "raisin:Folder".to_string(),  // Uses global type
    ..Default::default()
};
```

## Complete Example: Blog System

```rust
use raisin_models::nodes::types::NodeType;
use raisin_models::nodes::properties::schema::PropertyValueSchema;
use raisin_core::NodeTypeService;

async fn setup_blog_types(
    node_type_service: &NodeTypeService<S>,
    workspace: &str,
) -> Result<()> {
    // 1. Create base content type
    let base_content = NodeType {
        name: "BaseContent".to_string(),
        properties: Some(vec![
            PropertyValueSchema {
                name: Some("title".to_string()),
                property_type: PropertyType::String,
                required: Some(true),
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("slug".to_string()),
                property_type: PropertyType::String,
                required: Some(true),
                ..Default::default()
            },
        ]),
        versionable: Some(true),
        auditable: Some(true),
        ..Default::default()
    };
    node_type_service.put(workspace, base_content).await?;

    // 2. Create blog post type
    let blog_post = NodeType {
        name: "BlogPost".to_string(),
        extends: Some("BaseContent".to_string()),
        properties: Some(vec![
            PropertyValueSchema {
                name: Some("content".to_string()),
                property_type: PropertyType::String,
                required: Some(true),
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("excerpt".to_string()),
                property_type: PropertyType::String,
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("published_date".to_string()),
                property_type: PropertyType::Date,
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("category".to_string()),
                property_type: PropertyType::Reference,  // Automatically indexed!
                ..Default::default()
            },
        ]),
        publishable: Some(true),
        ..Default::default()
    };
    node_type_service.put(workspace, blog_post).await?;

    // 3. Create category type
    let category = NodeType {
        name: "Category".to_string(),
        properties: Some(vec![
            PropertyValueSchema {
                name: Some("name".to_string()),
                property_type: PropertyType::String,
                required: Some(true),
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("description".to_string()),
                property_type: PropertyType::String,
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    node_type_service.put(workspace, category).await?;

    // 4. Create blog container type with initial structure
    let blog = NodeType {
        name: "Blog".to_string(),
        allowed_children: vec!["raisin:Folder".to_string()],
        initial_structure: Some(InitialNodeStructure {
            children: Some(vec![
                InitialChild {
                    name: "posts".to_string(),
                    node_type: "raisin:Folder".to_string(),
                    properties: Some(hashmap!{
                        "title" => "Blog Posts".into(),
                    }),
                    children: None,
                    translations: None,
                    content_type: None,
                },
                InitialChild {
                    name: "categories".to_string(),
                    node_type: "raisin:Folder".to_string(),
                    properties: Some(hashmap!{
                        "title" => "Categories".into(),
                    }),
                    children: None,
                    translations: None,
                    content_type: None,
                },
            ]),
            properties: None,
        }),
        ..Default::default()
    };
    node_type_service.put(workspace, blog).await?;

    Ok(())
}
```

## Validation

RaisinDB validates nodes against their NodeType:

1. **Schema validation**: Properties match defined types
2. **Required fields**: All required properties are present
3. **Allowed children**: Child type is in `allowed_children`
4. **Required nodes**: All `required_nodes` exist
5. **Custom validation**: Regex patterns, min/max values, etc.

Example validation error:

```rust
// NodeType requires "title" field
let invalid_node = Node {
    name: "test".to_string(),
    node_type: "BlogPost".to_string(),
    properties: HashMap::new(),  // Missing required "title"!
    ..Default::default()
};

service.add_node(workspace, "/", invalid_node).await?;
// Error: Missing required property 'title'
```

## Best Practices

### 1. Plan Your Type Hierarchy

```
BaseContent (shared fields)
├── BlogPost (extends BaseContent)
├── NewsArticle (extends BaseContent)
└── Documentation (extends BaseContent)
```

### 2. Use Mixins for Cross-Cutting Concerns

Mixins are first-class entities created with `CREATE MIXIN` (or `is_mixin: Some(true)` in Rust). Common examples:

```sql
CREATE MIXIN 'myapp:SEO'         -- meta tags
  PROPERTIES (meta_title String, meta_description String);

CREATE MIXIN 'myapp:Timestamps'  -- created/updated dates
  PROPERTIES (created_at Date REQUIRED, updated_at Date REQUIRED);

CREATE MIXIN 'myapp:Author'      -- author information
  PROPERTIES (author_name String, author_email String);

CREATE MIXIN 'myapp:Taggable'    -- tagging support
  PROPERTIES (tags Array OF String);
```

### 3. Set Strict Mode for Critical Data

```rust
let customer_record = NodeType {
    strict: Some(true),  // Prevent accidental extra fields
    auditable: Some(true),  // Log all changes
    // ...
};
```

### 4. Use Initial Structure for Consistency

```rust
// Ensure all projects have the same folder structure
let project = NodeType {
    initial_structure: Some(/* standard folders */),
    // ...
};
```

### 5. Namespace Your Types

```rust
// Avoid naming conflicts
"myapp:BlogPost"
"myapp:Category"
"myapp:Author"
```

## Next Steps

- [Workspace Configuration](workspace-configuration.md) - Set up workspaces with allowed NodeTypes
- [Property Schemas](../guides/property-schemas.md) - Deep dive into property definitions
- [Node Operations](../guides/node-operations.md) - Working with node instances
