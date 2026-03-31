# Property Type Reference

Complete reference for all property types in RaisinDB.

## Available Property Types

```rust
pub enum PropertyType {
    String,          // Text values
    Number,          // Numeric values (integers and floats)
    Boolean,         // true/false
    Date,            // Timestamps
    URL,             // URL strings with validation
    Array,           // Lists of values
    Object,          // Nested structures
    Reference,       // References to other nodes
    NodeType,        // References to NodeType definitions
    Resource,        // File/media attachments
    Element,         // Content elements (see Block Types guide)
    Composite,       // Container for multiple elements
}
```

> **Note on PropertyValue variants**: While `PropertyType` defines the schema-level types,
> the runtime `PropertyValue` enum has additional variants for precise value representation:
> `Null`, `Integer(i64)`, `Float(f64)`, `Decimal(Decimal)`, `Url(RaisinUrl)`,
> `Vector(Vec<f32>)`, and `Geometry(GeoJson)`. The `Number` property type may deserialize
> to `Integer`, `Float`, or `Decimal` at runtime depending on the value.

## String

Text values of any length.

### Schema

```rust
PropertyValueSchema {
    name: Some("title".to_string()),
    property_type: PropertyType::String,
    required: Some(true),
    constraints: Some({
        let mut c = HashMap::new();
        c.insert("minLength".to_string(), PropertyValue::Number(1.0));
        c.insert("maxLength".to_string(), PropertyValue::Number(200.0));
        c.insert("pattern".to_string(),
            PropertyValue::String("^[A-Za-z0-9 ]+$".to_string()));
        c
    }),
    ..Default::default()
}
```

### Constraints

| Constraint | Type | Description | Example |
|------------|------|-------------|---------|
| `minLength` | Number | Minimum string length | `3.0` |
| `maxLength` | Number | Maximum string length | `100.0` |
| `pattern` | String | Regular expression | `"^[a-z]+$"` |
| `enum` | Array | Allowed values only | `["draft", "published"]` |

### Runtime Value

```rust
PropertyValue::String("Hello World".to_string())
```

### Usage

```rust
// Set value
node.properties.insert("title".to_string(),
    PropertyValue::String("My Article".to_string()));

// Get value
if let Some(PropertyValue::String(title)) = node.properties.get("title") {
    println!("Title: {}", title);
}
```

## Number

Numeric values (integers or floats).

### Schema

```rust
PropertyValueSchema {
    name: Some("price".to_string()),
    property_type: PropertyType::Number,
    required: Some(true),
    constraints: Some({
        let mut c = HashMap::new();
        c.insert("minimum".to_string(), PropertyValue::Number(0.0));
        c.insert("maximum".to_string(), PropertyValue::Number(999999.99));
        c.insert("multipleOf".to_string(), PropertyValue::Number(0.01));
        c
    }),
    ..Default::default()
}
```

### Constraints

| Constraint | Type | Description | Example |
|------------|------|-------------|---------|
| `minimum` | Number | Minimum value (inclusive) | `0.0` |
| `maximum` | Number | Maximum value (inclusive) | `100.0` |
| `exclusiveMinimum` | Number | Minimum value (exclusive) | `0.0` |
| `exclusiveMaximum` | Number | Maximum value (exclusive) | `100.0` |
| `multipleOf` | Number | Must be multiple of | `0.01`, `5.0` |

### Runtime Value

```rust
// Integers (no precision loss for large numbers)
PropertyValue::Integer(42)

// Floating point
PropertyValue::Float(42.5)

// Exact decimal (for financial calculations)
use rust_decimal::Decimal;
PropertyValue::Decimal(Decimal::new(1999, 2)) // 19.99
```

### Usage

```rust
// Set value (integer)
node.properties.insert("count".to_string(),
    PropertyValue::Integer(42));

// Set value (float)
node.properties.insert("price".to_string(),
    PropertyValue::Float(19.99));

// Get value - check both integer and float variants
match node.properties.get("price") {
    Some(PropertyValue::Float(price)) => println!("Price: ${:.2}", price),
    Some(PropertyValue::Integer(price)) => println!("Price: ${}", price),
    Some(PropertyValue::Decimal(price)) => println!("Price: ${}", price),
    _ => {},
}
```

## Boolean

True or false values.

### Schema

```rust
PropertyValueSchema {
    name: Some("isPublished".to_string()),
    property_type: PropertyType::Boolean,
    default: Some(PropertyValue::Boolean(false)),
    ..Default::default()
}
```

### Runtime Value

```rust
PropertyValue::Boolean(true)
```

### Usage

```rust
// Set value
node.properties.insert("isPublished".to_string(),
    PropertyValue::Boolean(true));

// Get value
if let Some(PropertyValue::Boolean(published)) = node.properties.get("isPublished") {
    if *published {
        println!("This article is published");
    }
}
```

## Date

Timestamps with timezone support.

### Schema

```rust
PropertyValueSchema {
    name: Some("publishedAt".to_string()),
    property_type: PropertyType::Date,
    ..Default::default()
}
```

### Constraints

| Constraint | Type | Description | Example |
|------------|------|-------------|---------|
| `minimum` | Date | Earliest allowed date | ISO 8601 string |
| `maximum` | Date | Latest allowed date | ISO 8601 string |

### Runtime Value

```rust
use chrono::Utc;

PropertyValue::Date(Utc::now())
```

### Usage

```rust
use chrono::Utc;

// Set value
node.properties.insert("publishedAt".to_string(),
    PropertyValue::Date(Utc::now()));

// Get value
if let Some(PropertyValue::Date(published)) = node.properties.get("publishedAt") {
    println!("Published: {}", published.format("%Y-%m-%d %H:%M:%S"));
}
```

## URL

URL strings with automatic validation.

### Schema

```rust
PropertyValueSchema {
    name: Some("website".to_string()),
    property_type: PropertyType::URL,
    constraints: Some({
        let mut c = HashMap::new();
        c.insert("pattern".to_string(),
            PropertyValue::String("^https?://".to_string()));  // Require http/https
        c
    }),
    ..Default::default()
}
```

### Runtime Value

```rust
use raisin_models::nodes::properties::value::RaisinUrl;

// Minimal URL
PropertyValue::Url(RaisinUrl::new("https://example.com"))

// Rich URL with metadata
PropertyValue::Url(
    RaisinUrl::new("https://example.com")
        .with_title("Example")
        .with_description("An example website")
        .external()  // Sets target="_blank" and rel="noopener"
)
```

### Usage

```rust
use raisin_models::nodes::properties::value::RaisinUrl;

// Set value
node.properties.insert("website".to_string(),
    PropertyValue::Url(RaisinUrl::new("https://example.com")));

// Get value
if let Some(PropertyValue::Url(url)) = node.properties.get("website") {
    println!("Website: {}", url.url);
    if let Some(title) = &url.title {
        println!("Title: {}", title);
    }
}
```

## Array

Lists of values with typed items.

### Schema

Simple array of strings:

```rust
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
        c.insert("minItems".to_string(), PropertyValue::Number(1.0));
        c.insert("maxItems".to_string(), PropertyValue::Number(10.0));
        c.insert("uniqueItems".to_string(), PropertyValue::Boolean(true));
        c
    }),
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

### Constraints

| Constraint | Type | Description | Example |
|------------|------|-------------|---------|
| `minItems` | Number | Minimum array length | `1.0` |
| `maxItems` | Number | Maximum array length | `10.0` |
| `uniqueItems` | Boolean | All items must be unique | `true` |

### Runtime Value

```rust
PropertyValue::Array(vec![
    PropertyValue::String("rust".to_string()),
    PropertyValue::String("database".to_string()),
])
```

### Usage

```rust
// Set value
node.properties.insert("tags".to_string(), PropertyValue::Array(vec![
    PropertyValue::String("technology".to_string()),
    PropertyValue::String("tutorial".to_string()),
]));

// Get value
if let Some(PropertyValue::Array(tags)) = node.properties.get("tags") {
    for tag in tags {
        if let PropertyValue::String(tag_str) = tag {
            println!("Tag: {}", tag_str);
        }
    }
}
```

## Object

Nested structures with defined schema.

### Schema

```rust
let mut address_structure = HashMap::new();

address_structure.insert("street".to_string(), PropertyValueSchema {
    property_type: PropertyType::String,
    required: Some(true),
    ..Default::default()
});

address_structure.insert("city".to_string(), PropertyValueSchema {
    property_type: PropertyType::String,
    required: Some(true),
    ..Default::default()
});

address_structure.insert("country".to_string(), PropertyValueSchema {
    property_type: PropertyType::String,
    required: Some(true),
    ..Default::default()
});

address_structure.insert("zipCode".to_string(), PropertyValueSchema {
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
    structure: Some(address_structure),
    allow_additional_properties: Some(false),  // Only allow defined properties
    ..Default::default()
}
```

### Runtime Value

```rust
let mut address = HashMap::new();
address.insert("street".to_string(), PropertyValue::String("123 Main St".to_string()));
address.insert("city".to_string(), PropertyValue::String("Springfield".to_string()));
address.insert("country".to_string(), PropertyValue::String("USA".to_string()));
address.insert("zipCode".to_string(), PropertyValue::String("12345".to_string()));

PropertyValue::Object(address)
```

### Usage

```rust
// Set value
let mut address = HashMap::new();
address.insert("street".to_string(),
    PropertyValue::String("123 Main St".to_string()));
address.insert("city".to_string(),
    PropertyValue::String("Springfield".to_string()));

node.properties.insert("address".to_string(), PropertyValue::Object(address));

// Get value
if let Some(PropertyValue::Object(address)) = node.properties.get("address") {
    if let Some(PropertyValue::String(city)) = address.get("city") {
        println!("City: {}", city);
    }
}
```

## Reference

References to other nodes in the repository.

### Schema

```rust
PropertyValueSchema {
    name: Some("author".to_string()),
    property_type: PropertyType::Reference,
    required: Some(true),
    meta: Some({
        let mut m = HashMap::new();
        m.insert("allowedTypes".to_string(), PropertyValue::Array(vec![
            PropertyValue::String("User".to_string()),
            PropertyValue::String("Author".to_string()),
        ]));
        m.insert("workspace".to_string(),
            PropertyValue::String("users".to_string()));
        m
    }),
    ..Default::default()
}
```

### Runtime Value

```rust
use raisin_models::nodes::properties::value::RaisinReference;

PropertyValue::Reference(RaisinReference {
    id: "user-123".to_string(),
    workspace: "users".to_string(),
    path: "/users/john-doe".to_string(),
})
```

### Usage

```rust
use raisin_models::nodes::properties::value::RaisinReference;

// Set value
node.properties.insert("author".to_string(),
    PropertyValue::Reference(RaisinReference {
        id: "user-123".to_string(),
        workspace: "users".to_string(),
        path: "/users/john-doe".to_string(),
    }));

// Get value
if let Some(PropertyValue::Reference(ref_val)) = node.properties.get("author") {
    println!("Author ID: {}", ref_val.id);
    println!("Author path: {}", ref_val.path);

    // Fetch the referenced node
    let author = node_service.get(&ref_val.workspace, &ref_val.id).await?;
}
```

### Reference Indexing

**References are automatically indexed by RaisinDB**, enabling fast bidirectional lookups. The reference index is updated synchronously during all CRUD operations (`put`, `delete`, `publish`, `unpublish`).

#### Index Structure

- **Forward Index**: Maps source node → list of referenced nodes (with property paths)
- **Reverse Index**: Maps target node → list of nodes that reference it
- **Dual Indexes**: Separate indexes for draft and published content

#### Automatic Indexing

```rust
// When you store a node with references, they're automatically indexed
let node = Node {
    name: "article-1".to_string(),
    node_type: "BlogPost".to_string(),
    properties: {
        let mut props = HashMap::new();
        props.insert("author".to_string(),
            PropertyValue::Reference(RaisinReference {
                id: "user-123".to_string(),
                workspace: "content".to_string(),
                path: "/users/john".to_string(),
            }));
        props
    },
    ..Default::default()
};

// References are indexed automatically
service.put("content", node).await?;

// No manual indexing needed!
```

### Querying References

Use the `ReferenceIndexRepository` to query references efficiently:

#### Find Nodes Referencing a Target

```rust
use raisin_storage::Storage;

let storage = /* your storage */;
let ref_index = storage.reference_index();

// Find all nodes that reference user-123
let referencing_nodes = ref_index
    .find_nodes_referencing("content", "user-123", false) // false = draft
    .await?;

println!("Found {} nodes referencing this user", referencing_nodes.len());
```

#### Find Outgoing References

```rust
// Find all references from article-1
let references = ref_index
    .find_outgoing_references("content", "article-1", false)
    .await?;

for (target_id, property_paths) in references {
    println!("References {} via properties: {:?}", target_id, property_paths);
}
```

### Draft vs Published Indexes

References are indexed separately for draft and published content:

```rust
// Query draft references
let draft_refs = ref_index
    .find_nodes_referencing("content", "user-123", false)
    .await?;

// Query published references
let published_refs = ref_index
    .find_nodes_referencing("content", "user-123", true)
    .await?;

// When you publish a node, references move from draft to published index automatically
service.publish("content", "/article-1").await?;
```

### Performance

- **O(1) lookup** for both forward and reverse queries
- **Batch-friendly**: Multiple references indexed in single operation
- **Storage-efficient**: Uses compact key encoding in RocksDB
- **Concurrent**: Thread-safe with RwLock (InMemory) or native RocksDB concurrency

### Reference Resolution

For convenience, use the `ReferenceResolver` service to automatically fetch and embed referenced nodes:

```rust
use raisin_core::ReferenceResolver;

let resolver = ReferenceResolver::new(
    storage, "my-tenant".to_string(), "my-repo".to_string(), "main".to_string(),
);

// Fetch the node
let node = service.get("content", "article-1").await?.unwrap();

// Resolve all references
let resolved = resolver.resolve("content", &node).await?;

// Access resolved references
for (ref_id, ref_node) in resolved.resolved_references {
    println!("Referenced node: {} ({})", ref_node.name, ref_id);
}
```

See the [Reference Resolution Guide](./reference-resolution.md) for complete documentation.

## NodeType

References to NodeType definitions (for meta-programming).

### Schema

```rust
PropertyValueSchema {
    name: Some("contentType".to_string()),
    property_type: PropertyType::NodeType,
    required: Some(true),
    ..Default::default()
}
```

### Runtime Value

```rust
PropertyValue::String("myapp:BlogPost".to_string())
```

### Usage

```rust
// Set value
node.properties.insert("contentType".to_string(),
    PropertyValue::String("myapp:Article".to_string()));

// Get value and fetch NodeType
if let Some(PropertyValue::String(type_name)) = node.properties.get("contentType") {
    let node_type = node_type_service.get("workspace", type_name).await?;
    // Use the NodeType schema...
}
```

## Resource

File and media attachments with metadata.

### Schema

```rust
PropertyValueSchema {
    name: Some("avatar".to_string()),
    property_type: PropertyType::Resource,
    meta: Some({
        let mut m = HashMap::new();
        m.insert("accept".to_string(),
            PropertyValue::String("image/*".to_string()));
        m.insert("maxSize".to_string(),
            PropertyValue::Integer(5242880));
        m.insert("aspectRatio".to_string(),
            PropertyValue::String("1:1".to_string()));
        m
    }),
    ..Default::default()
}
```

### Runtime Value

```rust
use raisin_models::nodes::properties::value::Resource;
use chrono::Utc;

PropertyValue::Resource(Resource {
    uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
    name: Some("avatar.png".to_string()),
    size: Some(125000),  // bytes
    mime_type: Some("image/png".to_string()),
    url: Some("/uploads/avatar.png".to_string()),
    metadata: Some({
        let mut meta = HashMap::new();
        meta.insert("width".to_string(), PropertyValue::Number(512.0));
        meta.insert("height".to_string(), PropertyValue::Number(512.0));
        meta
    }),
    is_loaded: Some(true),
    is_external: Some(false),
    created_at: Utc::now(),
    updated_at: Utc::now(),
})
```

### Usage

```rust
use raisin_models::nodes::properties::value::Resource;

// Set value
node.properties.insert("avatar".to_string(),
    PropertyValue::Resource(Resource {
        uuid: "resource-uuid".to_string(),
        name: Some("profile.jpg".to_string()),
        size: Some(245000),
        mime_type: Some("image/jpeg".to_string()),
        url: Some("/uploads/profile.jpg".to_string()),
        metadata: None,
        is_loaded: Some(true),
        is_external: Some(false),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }));

// Get value
if let Some(PropertyValue::Resource(resource)) = node.properties.get("avatar") {
    println!("File: {}", resource.name.as_ref().unwrap());
    println!("Size: {} bytes", resource.size.unwrap());
    println!("URL: {}", resource.url.as_ref().unwrap());
}
```

### Resource Structure

```rust
pub struct Resource {
    pub uuid: String,                              // Unique identifier
    pub name: Option<String>,                      // Filename
    pub size: Option<i64>,                         // Size in bytes
    pub mime_type: Option<String>,                 // MIME type (e.g., "image/png")
    pub url: Option<String>,                       // Access URL
    pub metadata: Option<HashMap<String, PropertyValue>>,  // Custom metadata
    pub is_loaded: Option<bool>,                   // Whether file is uploaded
    pub is_external: Option<bool>,                 // External URL vs uploaded
    pub created_at: DateTimeTimestamp,             // Upload timestamp
    pub updated_at: DateTimeTimestamp,             // Last modified
}
```

> `DateTimeTimestamp` is a type alias for `StorageTimestamp`. In JSON it serializes as RFC3339
> strings, and in binary formats (MessagePack) as i64 nanoseconds for storage efficiency.

## Element & Composite

See [Block Types Guide](block-types.md) for detailed documentation on structured content elements.

### Quick Overview

**Element**: A typed element within a composite structure.

```rust
PropertyValueSchema {
    name: Some("heroSection".to_string()),
    property_type: PropertyType::Element,
    ..Default::default()
}
```

**Composite**: A structured container for multiple elements.

```rust
PropertyValueSchema {
    name: Some("pageContent".to_string()),
    property_type: PropertyType::Composite,
    ..Default::default()
}
```

## Type Conversion

### Getting Property Values

Use pattern matching to extract values:

```rust
match node.properties.get("propertyName") {
    Some(PropertyValue::String(s)) => println!("String: {}", s),
    Some(PropertyValue::Integer(n)) => println!("Integer: {}", n),
    Some(PropertyValue::Float(n)) => println!("Float: {}", n),
    Some(PropertyValue::Decimal(n)) => println!("Decimal: {}", n),
    Some(PropertyValue::Boolean(b)) => println!("Boolean: {}", b),
    Some(PropertyValue::Date(d)) => println!("Date: {}", d),
    Some(PropertyValue::Array(arr)) => println!("Array with {} items", arr.len()),
    Some(PropertyValue::Object(obj)) => println!("Object with {} fields", obj.len()),
    Some(PropertyValue::Reference(r)) => println!("Reference to: {}", r.path),
    Some(PropertyValue::Url(u)) => println!("URL: {}", u.url),
    Some(PropertyValue::Resource(res)) => println!("Resource: {:?}", res.name),
    Some(PropertyValue::Null) => println!("Null"),
    _ => println!("Property not found or different type"),
}
```

### Helper Methods

```rust
impl Node {
    pub fn get_properties(&self) -> Properties<'_> {
        Properties::new(&self.properties)
    }
}

// Usage
let props = node.get_properties();
if let Some(title) = props.get("title") {
    // Work with property value
}
```

## Validation

Properties are validated when creating or updating nodes:

- **Type checking**: Value matches declared type
- **Required fields**: All required properties present
- **Constraints**: Values meet min/max, pattern, etc.
- **Unique**: No other node has same value (if unique=true)
- **Nested validation**: Object structure and array items validated

## Next Steps

- [Property Schemas Guide](property-schemas.md) - Defining schemas
- [Node System](../architecture/node-system.md) - NodeTypes and schemas
- [Nodes and Instances](nodes-and-instances.md) - Working with nodes
