# Nodes and Instances

Complete guide to working with Nodes in RaisinDB.

## What Are Nodes?

**Nodes** are instances of NodeTypes - the actual data in your system. Think of it this way:

- **NodeType** = Blueprint/Schema (defines what fields exist)
- **Node** = Instance/Record (actual data with values)

Example:
```rust
// NodeType defines the schema
let blog_post_type = NodeType {
    name: "BlogPost".to_string(),
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("title".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
        },
        PropertyValueSchema {
            name: Some("content".to_string()),
            property_type: PropertyType::String,
        },
    ]),
    ..Default::default()
};

// Node is an instance of that schema
let my_post = Node {
    name: "my-first-post".to_string(),
    node_type: "myapp:BlogPost".to_string(),
    properties: hashmap!{
        "title" => "Hello World".into(),
        "content" => "<p>Welcome to my blog!</p>".into(),
    },
    ..Default::default()
};
```

## Node Structure

Complete Node structure with all fields:

```rust
pub struct Node {
    // Core Identity
    pub id: String,                                    // Unique identifier (auto-generated)
    pub name: String,                                  // URL-safe name
    pub path: String,                                  // Full path (e.g., "/blog/my-post")
    pub node_type: String,                             // NodeType reference (e.g., "myapp:BlogPost")
    pub archetype: Option<String>,                     // Optional archetype for specialized rendering

    // Data
    pub properties: HashMap<String, PropertyValue>,    // Field values

    // Hierarchy
    pub children: Vec<String>,                         // Child node IDs (ordering by order_key)
    pub order_key: String,                             // Fractional index for sibling ordering (Base62)
    pub has_children: Option<bool>,                    // Computed field (populated at service layer)
    pub parent: Option<String>,                        // Parent node **name** (not full path!)

    // Versioning
    pub version: i32,                                  // Version number (starts at 1)

    // Timestamps
    pub created_at: Option<DateTime<Utc>>,            // Creation timestamp
    pub updated_at: Option<DateTime<Utc>>,            // Last update timestamp

    // Publishing (for publishable NodeTypes)
    pub published_at: Option<DateTime<Utc>>,          // Publication timestamp
    pub published_by: Option<String>,                  // User who published

    // User Tracking
    pub created_by: Option<String>,                    // User who created
    pub updated_by: Option<String>,                    // User who last updated
    pub owner_id: Option<String>,                      // Owner user ID

    // Multi-Language Support
    pub translations: Option<HashMap<String, PropertyValue>>,  // Translated values

    // Multi-Tenancy
    pub tenant_id: Option<String>,                     // Tenant identifier
    pub workspace: Option<String>,                     // Workspace name

    // Relations
    pub relations: Vec<RelationRef>,                   // Relations to other nodes
}
```

> **Important**: The `parent` field stores only the parent node's **name**, not its full path.
> For example, if a node's path is `/content/docs/page1`, its `parent` would be `"docs"`.
> Use `node.parent_path()` to get the full parent path.

## Creating Nodes

### Basic Node Creation

```rust
use raisin_core::NodeService;
use raisin_models::nodes::Node;
use std::collections::HashMap;

let service = NodeService::new(storage);

let node = Node {
    name: "homepage".to_string(),
    node_type: "myapp:Page".to_string(),
    properties: {
        let mut props = HashMap::new();
        props.insert("title".to_string(), "Welcome".into());
        props.insert("content".to_string(), "<h1>Hello</h1>".into());
        props
    },
    ..Default::default()
};

// Add node to workspace at given parent path
let created = service.add_node("content", "/", node).await?;

println!("Created node: {}", created.id);
println!("Path: {}", created.path);  // "/homepage"
println!("Version: {}", created.version);  // 1
```

### Node Creation in Folder

```rust
// Create a folder first
let folder = Node {
    name: "blog".to_string(),
    node_type: "raisin:Folder".to_string(),
    ..Default::default()
};
let created_folder = service.add_node("content", "/", folder).await?;

// Create a post inside the folder
let post = Node {
    name: "first-post".to_string(),
    node_type: "myapp:BlogPost".to_string(),
    properties: hashmap!{
        "title" => "My First Post".into(),
        "content" => "<p>Content here</p>".into(),
    },
    ..Default::default()
};

// Add to the folder's path
let created_post = service
    .add_node("content", &created_folder.path, post)
    .await?;

println!("Post path: {}", created_post.path);  // "/blog/first-post"
println!("Parent: {:?}", created_post.parent);  // Some(folder.id)
```

## Node Properties

### Setting Property Values

```rust
use raisin_models::nodes::properties::value::PropertyValue;

let mut properties = HashMap::new();

// String
properties.insert("title".to_string(),
    PropertyValue::String("Hello".to_string()));

// Number
properties.insert("price".to_string(),
    PropertyValue::Number(19.99));

// Boolean
properties.insert("published".to_string(),
    PropertyValue::Boolean(true));

// Date
use chrono::Utc;
properties.insert("publishedAt".to_string(),
    PropertyValue::Date(Utc::now()));

// Array
properties.insert("tags".to_string(),
    PropertyValue::Array(vec![
        PropertyValue::String("rust".to_string()),
        PropertyValue::String("tutorial".to_string()),
    ]));

// Object
let mut address = HashMap::new();
address.insert("street".to_string(), PropertyValue::String("123 Main St".to_string()));
address.insert("city".to_string(), PropertyValue::String("Springfield".to_string()));
properties.insert("address".to_string(), PropertyValue::Object(address));

// Reference to another node
use raisin_models::nodes::properties::value::RaisinReference;
properties.insert("author".to_string(),
    PropertyValue::Reference(RaisinReference {
        id: "user-123".to_string(),
        workspace: "users".to_string(),
        path: "/users/john".to_string(),
    }));

let node = Node {
    name: "complex-node".to_string(),
    node_type: "myapp:ComplexType".to_string(),
    properties,
    ..Default::default()
};
```

### Reading Property Values

```rust
// Get node
let node = service.get("content", &node_id).await?;

// Extract property values with pattern matching
if let Some(PropertyValue::String(title)) = node.properties.get("title") {
    println!("Title: {}", title);
}

if let Some(PropertyValue::Number(price)) = node.properties.get("price") {
    println!("Price: ${:.2}", price);
}

if let Some(PropertyValue::Boolean(published)) = node.properties.get("published") {
    if *published {
        println!("This is published");
    }
}

if let Some(PropertyValue::Array(tags)) = node.properties.get("tags") {
    for tag in tags {
        if let PropertyValue::String(tag_str) = tag {
            println!("Tag: {}", tag_str);
        }
    }
}

if let Some(PropertyValue::Reference(author_ref)) = node.properties.get("author") {
    println!("Author ID: {}", author_ref.id);
    // Fetch the referenced node
    let author = service.get(&author_ref.workspace, &author_ref.id).await?;
}
```

## Updating Nodes

### Update Node Properties

```rust
// Get the node
let mut node = service.get("content", &node_id).await?;

// Update properties
node.properties.insert("title".to_string(),
    PropertyValue::String("Updated Title".to_string()));

node.properties.insert("content".to_string(),
    PropertyValue::String("<p>New content</p>".to_string()));

// Save changes
let updated = service.update("content", node).await?;

println!("Updated at: {:?}", updated.updated_at);
println!("Version: {}", updated.version);  // Incremented if versionable
```

### Update with User Tracking

```rust
let mut node = service.get("content", &node_id).await?;

// Set who is making the update
node.updated_by = Some("user-456".to_string());

// Update properties
node.properties.insert("content".to_string(),
    PropertyValue::String("<p>New content</p>".to_string()));

let updated = service.update("content", node).await?;

println!("Updated by: {:?}", updated.updated_by);  // Some("user-456")
```

## Version Management

For nodes created from versionable NodeTypes:

```rust
// Create a versionable NodeType
let blog_type = NodeType {
    name: "BlogPost".to_string(),
    versionable: Some(true),  // Enable versioning
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("title".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
        },
    ]),
    ..Default::default()
};
node_type_service.put("content", blog_type).await?;

// Create node
let post = Node {
    name: "versioned-post".to_string(),
    node_type: "myapp:BlogPost".to_string(),
    properties: hashmap!{
        "title" => "First Draft".into(),
    },
    ..Default::default()
};
let created = service.add_node("content", "/", post).await?;
println!("Version: {}", created.version);  // 1

// Update creates new version
let mut updated_post = created.clone();
updated_post.properties.insert("title".to_string(), "Second Draft".into());
let v2 = service.update("content", updated_post).await?;
println!("Version: {}", v2.version);  // 2

// Update again
let mut updated_again = v2.clone();
updated_again.properties.insert("title".to_string(), "Final Version".into());
let v3 = service.update("content", updated_again).await?;
println!("Version: {}", v3.version);  // 3
```

## Publishing Workflow

For nodes created from publishable NodeTypes:

```rust
// Create publishable NodeType
let article_type = NodeType {
    name: "Article".to_string(),
    publishable: Some(true),  // Enable publishing
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("title".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
        },
        PropertyValueSchema {
            name: Some("content".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
        },
    ]),
    ..Default::default()
};
node_type_service.put("content", article_type).await?;

// Create node (unpublished)
let article = Node {
    name: "my-article".to_string(),
    node_type: "myapp:Article".to_string(),
    properties: hashmap!{
        "title" => "Draft Article".into(),
        "content" => "<p>Content goes here</p>".into(),
    },
    ..Default::default()
};
let created = service.add_node("content", "/", article).await?;
println!("Published: {:?}", created.published_at);  // None

// Publish the node
let mut article_to_publish = created.clone();
article_to_publish.published_at = Some(Utc::now());
article_to_publish.published_by = Some("user-123".to_string());

let published = service.update("content", article_to_publish).await?;
println!("Published at: {:?}", published.published_at);  // Some(timestamp)
println!("Published by: {:?}", published.published_by);  // Some("user-123")

// Check if published
fn is_published(node: &Node) -> bool {
    node.published_at.is_some()
}

if is_published(&published) {
    println!("Article is live!");
}

// Unpublish by setting to None
let mut unpublish = published.clone();
unpublish.published_at = None;
unpublish.published_by = None;
let unpublished = service.update("content", unpublish).await?;
```

## User Tracking

Track which users create and modify nodes:

```rust
// Create node with creator
let node = Node {
    name: "tracked-node".to_string(),
    node_type: "myapp:Document".to_string(),
    created_by: Some("user-123".to_string()),
    owner_id: Some("user-123".to_string()),
    properties: hashmap!{
        "title" => "My Document".into(),
    },
    ..Default::default()
};
let created = service.add_node("content", "/", node).await?;

println!("Created by: {:?}", created.created_by);  // Some("user-123")
println!("Owner: {:?}", created.owner_id);         // Some("user-123")

// Update by different user
let mut update = created.clone();
update.updated_by = Some("user-456".to_string());
update.properties.insert("title".to_string(), "Updated Document".into());

let updated = service.update("content", update).await?;
println!("Created by: {:?}", updated.created_by);  // Some("user-123") - unchanged
println!("Updated by: {:?}", updated.updated_by);  // Some("user-456")
println!("Owner: {:?}", updated.owner_id);         // Some("user-123") - unchanged
```

## Translation Support

For translatable properties:

```rust
// Create NodeType with translatable field
let page_type = NodeType {
    name: "Page".to_string(),
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("title".to_string()),
            property_type: PropertyType::String,
            is_translatable: Some(true),  // Mark as translatable
            required: Some(true),
        },
        PropertyValueSchema {
            name: Some("content".to_string()),
            property_type: PropertyType::String,
            is_translatable: Some(true),
        },
    ]),
    ..Default::default()
};
node_type_service.put("content", page_type).await?;

// Create node with default language (e.g., English)
let page = Node {
    name: "welcome".to_string(),
    node_type: "myapp:Page".to_string(),
    properties: hashmap!{
        "title" => "Welcome".into(),
        "content" => "<p>Welcome to our site</p>".into(),
    },
    ..Default::default()
};
let created = service.add_node("content", "/", page).await?;

// Add translations
let mut translated = created.clone();
let mut translations = HashMap::new();

// Spanish translation
translations.insert("es".to_string(), PropertyValue::Object({
    let mut es = HashMap::new();
    es.insert("title".to_string(), PropertyValue::String("Bienvenido".to_string()));
    es.insert("content".to_string(),
        PropertyValue::String("<p>Bienvenido a nuestro sitio</p>".to_string()));
    es
}));

// French translation
translations.insert("fr".to_string(), PropertyValue::Object({
    let mut fr = HashMap::new();
    fr.insert("title".to_string(), PropertyValue::String("Bienvenue".to_string()));
    fr.insert("content".to_string(),
        PropertyValue::String("<p>Bienvenue sur notre site</p>".to_string()));
    fr
}));

translated.translations = Some(translations);
let with_translations = service.update("content", translated).await?;

// Access translations
if let Some(translations) = &with_translations.translations {
    if let Some(PropertyValue::Object(es_trans)) = translations.get("es") {
        if let Some(PropertyValue::String(title)) = es_trans.get("title") {
            println!("Spanish title: {}", title);  // "Bienvenido"
        }
    }
}
```

## Path and Hierarchy

### Understanding Paths

```rust
// Paths are hierarchical
let folder = Node {
    name: "docs".to_string(),
    node_type: "raisin:Folder".to_string(),
    ..Default::default()
};
let folder_created = service.add_node("content", "/", folder).await?;
// Path: "/docs"

let subfolder = Node {
    name: "guides".to_string(),
    node_type: "raisin:Folder".to_string(),
    ..Default::default()
};
let sub_created = service.add_node("content", &folder_created.path, subfolder).await?;
// Path: "/docs/guides"

let page = Node {
    name: "getting-started".to_string(),
    node_type: "raisin:Page".to_string(),
    ..Default::default()
};
let page_created = service.add_node("content", &sub_created.path, page).await?;
// Path: "/docs/guides/getting-started"
```

### Working with Children

```rust
// Get node with children
let folder = service.get("content", &folder_id).await?;

// List child IDs
for child_id in &folder.children {
    let child = service.get("content", child_id).await?;
    println!("Child: {} ({})", child.name, child.path);
}

// List children by path
let children = service.list_children("content", "/docs").await?;
for child in children {
    println!("- {} ({})", child.name, child.node_type);
}
```

### Working with Parent

```rust
let node = service.get("content", &node_id).await?;

if let Some(parent_id) = &node.parent {
    let parent = service.get("content", parent_id).await?;
    println!("Parent: {} ({})", parent.name, parent.path);
}
```

## Querying Nodes

### Get by ID

```rust
let node = service.get("content", &node_id).await?;
println!("Found: {}", node.name);
```

### Get by Path

```rust
let node = service.get_by_path("content", "/blog/my-post").await?;
if let Some(found) = node {
    println!("Found: {}", found.name);
}
```

### List All Nodes

```rust
let all_nodes = service.list_all("content").await?;
println!("Total nodes: {}", all_nodes.len());

for node in all_nodes {
    println!("- {} at {}", node.name, node.path);
}
```

### List Children

```rust
let children = service.list_children("content", "/blog").await?;
for child in children {
    println!("- {}", child.name);
}
```

## Deleting Nodes

```rust
// Delete a node
service.delete("content", &node_id).await?;
println!("Deleted node: {}", node_id);

// Delete by path
let node = service.get_by_path("content", "/blog/old-post").await?;
if let Some(found) = node {
    service.delete("content", &found.id).await?;
    println!("Deleted: {}", found.path);
}
```

**Note**: Deleting a folder will also delete all its children recursively.

## Multi-Tenancy Context

In multi-tenant mode, nodes are automatically scoped to tenant:

```rust
use raisin_storage::TenantContext;

// Tenant A
let ctx_a = TenantContext::new("tenant-a", "production");
let service_a = NodeService::scoped(storage.clone(), ctx_a);

let node_a = Node {
    name: "page".to_string(),
    node_type: "raisin:Page".to_string(),
    ..Default::default()
};
let created_a = service_a.add_node("content", "/", node_a).await?;
// tenant_id field is automatically set to "tenant-a"

// Tenant B
let ctx_b = TenantContext::new("tenant-b", "production");
let service_b = NodeService::scoped(storage.clone(), ctx_b);

let node_b = Node {
    name: "page".to_string(),
    node_type: "raisin:Page".to_string(),
    ..Default::default()
};
let created_b = service_b.add_node("content", "/", node_b).await?;
// tenant_id field is automatically set to "tenant-b"

// These are completely isolated
// Tenant A cannot access Tenant B's nodes
```

## Best Practices

### 1. Automatic Validation

**NodeService automatically validates nodes** on `add_node()` and `put()` operations. You don't need to manually validate:

```rust
// ✅ Validation is automatic
let node = Node {
    name: "my-article".to_string(),
    node_type: "myapp:Article".to_string(),
    properties: hashmap!{
        "title" => "My Title".into(),
    },
    ..Default::default()
};

// This automatically validates:
// - NodeType exists
// - Required properties are present
// - Strict mode compliance (no undefined properties)
// - Unique property constraints
let result = service.add_node("content", "/", node).await;

match result {
    Ok(created) => println!("Node created: {}", created.id),
    Err(e) => {
        // Handle validation errors
        match e {
            raisin_error::Error::Validation(msg) => {
                println!("Validation failed: {}", msg);
            },
            raisin_error::Error::NotFound(msg) => {
                println!("Not found: {}", msg);
            },
            _ => println!("Error: {}", e),
        }
    }
}
```

**What gets validated automatically:**
- **NodeType existence**: The NodeType must exist in the workspace
- **Required properties**: All required properties must be present
- **Strict mode**: If strict=true, no undefined properties allowed
- **Unique constraints**: Properties marked as unique must be unique across workspace
- **Parent existence**: Parent path must exist (unless using `add_deep_node()`)

### 2. Use Type-Safe Property Access

```rust
// Create helper functions for common properties
fn get_title(node: &Node) -> Option<&str> {
    match node.properties.get("title") {
        Some(PropertyValue::String(s)) => Some(s),
        _ => None,
    }
}

fn get_published(node: &Node) -> bool {
    match node.properties.get("published") {
        Some(PropertyValue::Boolean(b)) => *b,
        _ => false,
    }
}
```

### 3. Handle Validation Errors

```rust
// Proper error handling for node creation
async fn create_article_safe(
    service: &NodeService<S>,
    workspace: &str,
    title: &str,
) -> Result<Node, String> {
    let node = Node {
        name: slugify(title),
        node_type: "myapp:Article".to_string(),
        properties: hashmap!{
            "title" => title.into(),
        },
        ..Default::default()
    };

    service.add_node(workspace, "/", node).await
        .map_err(|e| match e {
            raisin_error::Error::Validation(msg) => {
                format!("Invalid article: {}", msg)
            },
            raisin_error::Error::NotFound(msg) => {
                format!("Not found: {}", msg)
            },
            _ => format!("Failed to create article: {}", e),
        })
}
```

### 4. Track User Actions

```rust
// Always set user tracking fields
async fn create_node_with_user(
    service: &NodeService<S>,
    workspace: &str,
    parent_path: &str,
    mut node: Node,
    user_id: &str,
) -> Result<Node> {
    node.created_by = Some(user_id.to_string());
    node.owner_id = Some(user_id.to_string());

    service.add_node(workspace, parent_path, node).await
}

async fn update_node_with_user(
    service: &NodeService<S>,
    workspace: &str,
    mut node: Node,
    user_id: &str,
) -> Result<Node> {
    node.updated_by = Some(user_id.to_string());

    service.update(workspace, node).await
}
```

### 5. Use Meaningful Names

```rust
// ✅ Good - descriptive, URL-safe names
"getting-started"
"user-profile"
"blog-post-2024"

// ❌ Avoid - not descriptive or not URL-safe
"page1"
"My Document"  // spaces
"user_profile"  // underscores (use hyphens)
```

## Common Patterns

### Content Creation Pipeline

```rust
async fn create_article(
    service: &NodeService<S>,
    title: &str,
    content: &str,
    author_id: &str,
) -> Result<Node> {
    let node = Node {
        name: slugify(title),  // Convert to URL-safe name
        node_type: "myapp:Article".to_string(),
        properties: hashmap!{
            "title" => title.into(),
            "content" => content.into(),
        },
        created_by: Some(author_id.to_string()),
        owner_id: Some(author_id.to_string()),
        ..Default::default()
    };

    service.add_node("content", "/articles", node).await
}

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .replace(" ", "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}
```

### Publishing Workflow

```rust
async fn publish_node(
    service: &NodeService<S>,
    workspace: &str,
    node_id: &str,
    user_id: &str,
) -> Result<Node> {
    let mut node = service.get(workspace, node_id).await?;

    node.published_at = Some(Utc::now());
    node.published_by = Some(user_id.to_string());
    node.updated_by = Some(user_id.to_string());

    service.update(workspace, node).await
}

async fn unpublish_node(
    service: &NodeService<S>,
    workspace: &str,
    node_id: &str,
    user_id: &str,
) -> Result<Node> {
    let mut node = service.get(workspace, node_id).await?;

    node.published_at = None;
    node.published_by = None;
    node.updated_by = Some(user_id.to_string());

    service.update(workspace, node).await
}
```

## Next Steps

- [Property Schemas Guide](property-schemas.md) - Defining property schemas
- [Property Type Reference](property-reference.md) - All property types
- [Node System Architecture](../architecture/node-system.md) - NodeType system
- [Workspace Configuration](../architecture/workspace-configuration.md) - Workspace setup
