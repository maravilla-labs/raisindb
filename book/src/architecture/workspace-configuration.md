# Workspace Configuration

How to properly create and configure workspaces in RaisinDB.

## What is a Workspace?

A **Workspace** is a configured container that defines:
- Which **NodeTypes** are allowed
- Which **NodeTypes** can be root nodes
- Optional ordering of root children
- Dependencies on other workspaces

**Important**: Workspaces are **NOT** created automatically. You must explicitly create and configure them before adding nodes.

## Workspace Structure

```rust
pub struct Workspace {
    pub name: String,                       // Workspace identifier
    pub description: Option<String>,        // Human-readable description
    pub allowed_node_types: Vec<String>,    // REQUIRED: NodeTypes allowed in this workspace
    pub allowed_root_node_types: Vec<String>,  // REQUIRED: NodeTypes that can be at root level
    pub depends_on: Vec<String>,            // Dependencies on other workspaces
    pub initial_structure: Option<InitialNodeStructure>,  // Auto-created root nodes
    pub created_at: StorageTimestamp,        // Creation timestamp
    pub updated_at: Option<StorageTimestamp>, // Last update timestamp
    pub config: WorkspaceConfig,            // Branch and NodeType version config
}

pub struct WorkspaceConfig {
    pub default_branch: String,             // Default branch (defaults to "main")
    pub node_type_pins: HashMap<String, Option<HLC>>,  // NodeType revision pinning
}
```

## The Setup Flow

**Correct Order**:
1. Create **NodeTypes** (define schemas)
2. Create **Workspaces** (configure what's allowed)
3. Create **Nodes** (actual data)

```rust
// ❌ WRONG: This will fail!
service.add_node("content", "/", node).await?;
// Error: Workspace 'content' does not exist

// ✅ CORRECT: Create workspace first
let workspace = Workspace {
    name: "content".to_string(),
    allowed_node_types: vec!["raisin:Page".to_string()],
    allowed_root_node_types: vec!["raisin:Folder".to_string()],
    ..Default::default()
};
workspace_service.put(workspace).await?;

// Now you can add nodes
service.add_node("content", "/", node).await?;  // Works!
```

## Creating a Workspace

### Basic Workspace

```rust
use raisin_models::workspace::Workspace;
use raisin_core::WorkspaceService;

let workspace_service = WorkspaceService::new(storage);

let content_workspace = Workspace {
    name: "content".to_string(),
    description: Some("Website content and pages".to_string()),
    allowed_node_types: vec![
        "raisin:Folder".to_string(),
        "raisin:Page".to_string(),
        "myapp:BlogPost".to_string(),
    ],
    allowed_root_node_types: vec![
        "raisin:Folder".to_string(),  // Only folders can be at root
    ],
    depends_on: vec![],
    ..Default::default()
};

workspace_service.put(content_workspace).await?;
```

### Workspace with Initial Structure

Automatically create root-level nodes when the workspace is set up:

```rust
let website_workspace = Workspace {
    name: "website".to_string(),
    allowed_node_types: vec![
        "myapp:Header".to_string(),
        "myapp:Footer".to_string(),
        "raisin:Folder".to_string(),
    ],
    allowed_root_node_types: vec![
        "myapp:Header".to_string(),
        "myapp:Footer".to_string(),
        "raisin:Folder".to_string(),
    ],
    initial_structure: Some(InitialNodeStructure {
        children: Some(vec![
            InitialChild { name: "header".to_string(), node_type: "myapp:Header".to_string(), ..Default::default() },
            InitialChild { name: "navigation".to_string(), node_type: "raisin:Folder".to_string(), ..Default::default() },
            InitialChild { name: "pages".to_string(), node_type: "raisin:Folder".to_string(), ..Default::default() },
            InitialChild { name: "footer".to_string(), node_type: "myapp:Footer".to_string(), ..Default::default() },
        ]),
        properties: None,
    }),
    ..Default::default()
};

workspace_service.put(website_workspace).await?;
```

### Workspace with Dependencies

One workspace can depend on another:

```rust
// Base workspace
let media_workspace = Workspace {
    name: "media".to_string(),
    allowed_node_types: vec![
        "raisin:Asset".to_string(),
        "raisin:Folder".to_string(),
    ],
    allowed_root_node_types: vec!["raisin:Folder".to_string()],
    ..Default::default()
};
workspace_service.put(media_workspace).await?;

// Dependent workspace
let content_workspace = Workspace {
    name: "content".to_string(),
    allowed_node_types: vec![
        "myapp:Article".to_string(),
        "raisin:Folder".to_string(),
    ],
    allowed_root_node_types: vec!["raisin:Folder".to_string()],
    depends_on: vec!["media".to_string()],  // Depends on media workspace
    ..Default::default()
};
workspace_service.put(content_workspace).await?;

// Now articles can reference media assets
```

## Allowed Node Types

The `allowed_node_types` field controls which NodeTypes can exist in the workspace:

```rust
let blog_workspace = Workspace {
    name: "blog".to_string(),
    allowed_node_types: vec![
        "raisin:Folder".to_string(),
        "myapp:BlogPost".to_string(),
        "myapp:Category".to_string(),
        "myapp:Tag".to_string(),
    ],
    // ...
};

// ✅ Can create these types in "blog" workspace
service.add_node("blog", "/", folder_node).await?;  // raisin:Folder - OK
service.add_node("blog", "/", post_node).await?;    // myapp:BlogPost - OK

// ❌ Cannot create other types
service.add_node("blog", "/", product_node).await?;  // myapp:Product - Error!
```

## Allowed Root Node Types

The `allowed_root_node_types` field controls which NodeTypes can be at the root level (`/`):

```rust
let organized_workspace = Workspace {
    name: "organized".to_string(),
    allowed_node_types: vec![
        "raisin:Folder".to_string(),
        "myapp:Document".to_string(),
    ],
    allowed_root_node_types: vec![
        "raisin:Folder".to_string(),  // Only folders allowed at root
    ],
    ..Default::default()
};

// ✅ Can create folder at root
let folder = Node {
    node_type: "raisin:Folder".to_string(),
    // ...
};
service.add_node("organized", "/", folder).await?;  // OK

// ❌ Cannot create document at root
let doc = Node {
    node_type: "myapp:Document".to_string(),
    // ...
};
service.add_node("organized", "/", doc).await?;  // Error!

// ✅ But can create document inside folder
service.add_node("organized", "/my-folder", doc).await?;  // OK
```

## Common Workspace Patterns

### CMS Content Workspace

```rust
let cms_content = Workspace {
    name: "content".to_string(),
    description: Some("CMS content pages".to_string()),
    allowed_node_types: vec![
        "raisin:Folder".to_string(),
        "raisin:Page".to_string(),
        "cms:Article".to_string(),
        "cms:LandingPage".to_string(),
    ],
    allowed_root_node_types: vec![
        "raisin:Folder".to_string(),  // Organize with folders
    ],
    initial_structure: Some(InitialNodeStructure {
        children: Some(vec![
            InitialChild { name: "home".to_string(), node_type: "raisin:Folder".to_string(), ..Default::default() },
            InitialChild { name: "blog".to_string(), node_type: "raisin:Folder".to_string(), ..Default::default() },
            InitialChild { name: "about".to_string(), node_type: "raisin:Folder".to_string(), ..Default::default() },
        ]),
        properties: None,
    }),
    ..Default::default()
};
```

### DAM (Digital Asset Management) Workspace

```rust
let dam_workspace = Workspace {
    name: "dam".to_string(),
    description: Some("Digital assets and media files".to_string()),
    allowed_node_types: vec![
        "raisin:Folder".to_string(),
        "raisin:Asset".to_string(),
        "dam:Image".to_string(),
        "dam:Video".to_string(),
        "dam:Document".to_string(),
    ],
    allowed_root_node_types: vec![
        "raisin:Folder".to_string(),
    ],
    initial_structure: Some(InitialNodeStructure {
        children: Some(vec![
            InitialChild { name: "images".to_string(), node_type: "raisin:Folder".to_string(), ..Default::default() },
            InitialChild { name: "videos".to_string(), node_type: "raisin:Folder".to_string(), ..Default::default() },
            InitialChild { name: "documents".to_string(), node_type: "raisin:Folder".to_string(), ..Default::default() },
        ]),
        properties: None,
    }),
    ..Default::default()
};
```

### E-Commerce Product Workspace

```rust
let products_workspace = Workspace {
    name: "products".to_string(),
    description: Some("Product catalog".to_string()),
    allowed_node_types: vec![
        "raisin:Folder".to_string(),
        "ecommerce:Product".to_string(),
        "ecommerce:Category".to_string(),
        "ecommerce:Variant".to_string(),
    ],
    allowed_root_node_types: vec![
        "ecommerce:Category".to_string(),  // Categories at root
    ],
    depends_on: vec!["dam".to_string()],  // Products reference images
    ..Default::default()
};
```

### Customer Data Workspace

```rust
let customers_workspace = Workspace {
    name: "customers".to_string(),
    description: Some("Customer records and CRM data".to_string()),
    allowed_node_types: vec![
        "raisin:Folder".to_string(),
        "crm:Customer".to_string(),
        "crm:Order".to_string(),
        "crm:Invoice".to_string(),
    ],
    allowed_root_node_types: vec![
        "raisin:Folder".to_string(),
    ],
    ..Default::default()
};
```

## Complete Setup Example

Here's a complete example showing the proper order:

```rust
use raisin_core::{NodeService, NodeTypeService, WorkspaceService};
use raisin_rocksdb;
use std::sync::Arc;

async fn setup_blog_system() -> Result<()> {
    // Initialize storage
    let db = Arc::new(raisin_rocksdb::open_db("./data")?);

    // Create services
    let node_type_service = NodeTypeService::new(db.clone());
    let workspace_service = WorkspaceService::new(db.clone());
    let node_service = NodeService::new(db);

    // Step 1: Create NodeTypes
    let blog_post_type = NodeType {
        name: "BlogPost".to_string(),
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
                ..Default::default()
            },
        ]),
        publishable: Some(true),
        versionable: Some(true),
        ..Default::default()
    };
    node_type_service.put("blog", blog_post_type).await?;

    let category_type = NodeType {
        name: "Category".to_string(),
        properties: Some(vec![
            PropertyValueSchema {
                name: Some("name".to_string()),
                property_type: PropertyType::String,
                required: Some(true),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    node_type_service.put("blog", category_type).await?;

    // Step 2: Create Workspace
    let blog_workspace = Workspace {
        name: "blog".to_string(),
        description: Some("Blog content and organization".to_string()),
        allowed_node_types: vec![
            "raisin:Folder".to_string(),
            "myapp:BlogPost".to_string(),
            "myapp:Category".to_string(),
        ],
        allowed_root_node_types: vec![
            "raisin:Folder".to_string(),
        ],
        initial_structure: Some(InitialNodeStructure {
            children: Some(vec![
                InitialChild { name: "posts".to_string(), node_type: "raisin:Folder".to_string(), ..Default::default() },
                InitialChild { name: "categories".to_string(), node_type: "raisin:Folder".to_string(), ..Default::default() },
            ]),
            properties: None,
        }),
        ..Default::default()
    };
    workspace_service.put(blog_workspace).await?;

    // Step 3: Create Nodes
    let posts_folder = Node {
        name: "posts".to_string(),
        node_type: "raisin:Folder".to_string(),
        ..Default::default()
    };
    node_service.add_node("blog", "/", posts_folder).await?;

    let first_post = Node {
        name: "hello-world".to_string(),
        node_type: "myapp:BlogPost".to_string(),
        properties: hashmap!{
            "title" => "Hello World".into(),
            "content" => "My first blog post!".into(),
        },
        ..Default::default()
    };
    node_service.add_node("blog", "/posts", first_post).await?;

    println!("Blog system setup complete!");
    Ok(())
}
```

## Updating Workspaces

You can update workspace configuration:

```rust
// Get existing workspace
let mut workspace = workspace_service.get("content").await?
    .expect("Workspace not found");

// Add new allowed type
workspace.allowed_node_types.push("myapp:NewType".to_string());
workspace.updated_at = Some(Utc::now());

// Save changes
workspace_service.put(workspace).await?;
```

## Listing Workspaces

```rust
let all_workspaces = workspace_service.list().await?;

for ws in all_workspaces {
    println!("Workspace: {}", ws.name);
    println!("  Allowed types: {:?}", ws.allowed_node_types);
    println!("  Root types: {:?}", ws.allowed_root_node_types);
}
```

## Multi-Tenant Workspaces

In multi-tenant mode, workspaces are scoped to tenants:

```rust
let ctx = TenantContext::new("acme", "production");
let scoped_service = WorkspaceService::scoped(storage, ctx);

// Create workspace for this tenant
let workspace = Workspace {
    name: "content".to_string(),
    allowed_node_types: vec!["raisin:Page".to_string()],
    allowed_root_node_types: vec!["raisin:Folder".to_string()],
    ..Default::default()
};
scoped_service.put(workspace).await?;

// Each tenant can have different workspace configurations
```

## Validation

RaisinDB validates:

1. **NodeType exists**: All types in `allowed_node_types` must exist
2. **Root types are subset**: `allowed_root_node_types` must be subset of `allowed_node_types`
3. **Dependencies exist**: Referenced workspaces in `depends_on` must exist
4. **Node creation**: Nodes must have allowed NodeType
5. **Root placement**: Root nodes must have allowed root NodeType

## Best Practices

### 1. Plan Your Workspaces

Think about how you'll organize data:
- **content**: Website pages and articles
- **dam**: Media and assets
- **products**: E-commerce products
- **customers**: Customer data
- **settings**: Configuration

### 2. Use Descriptive Names

```rust
// ✅ Good
"blog-content"
"product-catalog"
"customer-records"

// ❌ Avoid
"workspace1"
"ws"
"data"
```

### 3. Be Restrictive

Only allow NodeTypes you actually need:

```rust
// ✅ Good - specific
allowed_node_types: vec![
    "raisin:Folder".to_string(),
    "myapp:Article".to_string(),
]

// ❌ Too permissive
allowed_node_types: vec![
    "global:*".to_string(),  // Allows everything!
]
```

### 4. Control Root Access

Prevent clutter at root level:

```rust
// ✅ Good - organized structure required
allowed_root_node_types: vec![
    "raisin:Folder".to_string(),  // Must organize in folders
]

// ❌ Allows chaos
allowed_root_node_types: vec![
    "raisin:Folder".to_string(),
    "myapp:Article".to_string(),
    "myapp:Product".to_string(),
    // ... everything allowed at root
]
```

### 5. Document Dependencies

```rust
let content_workspace = Workspace {
    name: "content".to_string(),
    description: Some(
        "Website content. References media from 'dam' workspace.".to_string()
    ),
    depends_on: vec!["dam".to_string()],
    // ...
};
```

## Troubleshooting

### Error: "Workspace does not exist"

```rust
// ❌ Forgot to create workspace
service.add_node("content", "/", node).await?;
// Error: Workspace 'content' does not exist

// ✅ Create workspace first
workspace_service.put(content_workspace).await?;
service.add_node("content", "/", node).await?;  // Works
```

### Error: "NodeType not allowed in workspace"

```rust
// Workspace only allows folders and pages
let workspace = Workspace {
    allowed_node_types: vec![
        "raisin:Folder".to_string(),
        "raisin:Page".to_string(),
    ],
    // ...
};

// ❌ Trying to add different type
let product = Node {
    node_type: "myapp:Product".to_string(),  // Not in allowed_node_types!
    // ...
};
service.add_node("content", "/", product).await?;
// Error: NodeType 'myapp:Product' not allowed in workspace 'content'

// ✅ Add to allowed types first
workspace.allowed_node_types.push("myapp:Product".to_string());
workspace_service.put(workspace).await?;
```

### Error: "NodeType not allowed at root"

```rust
// Only folders allowed at root
let workspace = Workspace {
    allowed_node_types: vec!["raisin:Folder".to_string(), "raisin:Page".to_string()],
    allowed_root_node_types: vec!["raisin:Folder".to_string()],
    // ...
};

// ❌ Trying to add page at root
let page = Node {
    node_type: "raisin:Page".to_string(),
    // ...
};
service.add_node("content", "/", page).await?;
// Error: NodeType 'global:Page' not allowed at root

// ✅ Add inside folder
service.add_node("content", "/my-folder", page).await?;  // Works
```

## Next Steps

- [Node System](node-system.md) - Understanding NodeTypes and schemas
- [Workspaces](workspaces.md) - Workspace concepts and multi-tenancy
- [Quick Start](../getting-started/quickstart.md) - Complete setup tutorial
