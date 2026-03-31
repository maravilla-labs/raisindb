# Workspaces

Understanding the workspace concept in RaisinDB.

## What Are Workspaces?

**Workspaces are app-specific organizational units** for grouping related data within your application. They are completely independent of multi-tenancy and serve to organize different types of content.

Think of workspaces as "folders" or "databases" within your application:

- **`content`** - Website pages, blog posts, articles
- **`dam`** - Digital asset management (images, videos, files)
- **`customers`** - Customer records and CRM data
- **`contracts`** - Legal documents and agreements
- **`products`** - E-commerce product catalog

You define workspaces based on your application's needs. RaisinDB doesn't impose any structure - they're just strings you use to organize your data.

## Workspaces vs Tenancy

**Important**: Workspaces and tenant isolation are orthogonal concepts:

| Concept | Purpose | Example |
|---------|---------|---------|
| **Workspace** | Organize data types | "content", "dam", "customers" |
| **Tenant** | Isolate customer data | "acme-corp", "techco" |
| **Deployment** | Isolate environments | "production", "staging", "preview" |

### Single-Tenant Mode

In single-tenant mode, you have one application with multiple workspaces:

```rust
let service = NodeService::new(storage);

// Create nodes in different workspaces
service.add_node("content", "/", page_node).await?;
service.add_node("dam", "/", image_node).await?;
service.add_node("customers", "/", customer_node).await?;
```

Storage structure:
```
RocksDB
├── nodes:content:node-1        # Website content
├── nodes:dam:asset-1           # Digital assets
└── nodes:customers:cust-1      # Customer records
```

### Multi-Tenant Mode

In multi-tenant mode, **each tenant has their own set of workspaces**:

```rust
// Tenant: acme-corp
let acme_ctx = TenantContext::new("acme", "production");
let acme_service = NodeService::scoped(storage.clone(), acme_ctx);

acme_service.add_node("content", "/", page_node).await?;
acme_service.add_node("dam", "/", logo_node).await?;

// Tenant: techco
let techco_ctx = TenantContext::new("techco", "production");
let techco_service = NodeService::scoped(storage.clone(), techco_ctx);

techco_service.add_node("content", "/", page_node).await?;
techco_service.add_node("dam", "/", banner_node).await?;
```

Storage structure:
```
RocksDB
├── /acme/production/
│   ├── nodes:content:node-1    # Acme's website content
│   ├── nodes:dam:logo          # Acme's digital assets
│   └── nodes:customers:cust-1  # Acme's customers
├── /techco/production/
│   ├── nodes:content:node-1    # TechCo's website content
│   ├── nodes:dam:banner        # TechCo's digital assets
│   └── nodes:customers:cust-1  # TechCo's customers
```

Each tenant has isolated "content", "dam", and "customers" workspaces.

## Repository Concept

In multi-tenant mode, a **repository** is the combination of:

```
repository = tenant_id + deployment_key
```

Examples:

| Tenant ID | Deployment | Repository |
|-----------|------------|------------|
| `acme` | `production` | acme's production repository |
| `acme` | `preview` | acme's preview repository |
| `acme` | `staging` | acme's staging repository |
| `techco` | `production` | techco's production repository |

Each repository contains the same workspaces (content, dam, etc.), but the data is completely isolated.

### Project-Based Tenancy

You can use this pattern for project-based isolation:

```rust
// Project A - staging environment
let ctx = TenantContext::new("projecta", "staging");
let service = NodeService::scoped(storage.clone(), ctx);
service.add_node("content", "/", node).await?;

// Project A - production environment
let ctx = TenantContext::new("projecta", "production");
let service = NodeService::scoped(storage.clone(), ctx);
service.add_node("content", "/", node).await?;

// Project B - its own repository
let ctx = TenantContext::new("projectb", "repository");
let service = NodeService::scoped(storage.clone(), ctx);
service.add_node("content", "/", node).await?;
```

Storage structure:
```
RocksDB
├── /projecta/staging/
│   ├── nodes:content:...
│   └── nodes:dam:...
├── /projecta/production/
│   ├── nodes:content:...
│   └── nodes:dam:...
├── /projectb/repository/
│   ├── nodes:content:...
│   └── nodes:dam:...
```

## Common Workspace Patterns

### CMS / Headless CMS

```rust
// Typical CMS workspaces
let workspaces = ["content", "media", "users", "settings"];

// Create a page
service.add_node("content", "/blog", page).await?;

// Upload an image
service.add_node("media", "/images", image).await?;
```

### E-Commerce Platform

```rust
// E-commerce workspaces
let workspaces = ["products", "orders", "customers", "inventory"];

// Add a product
service.add_node("products", "/electronics", product).await?;

// Create an order
service.add_node("orders", "/2024", order).await?;
```

### Document Management System

```rust
// Document management workspaces
let workspaces = ["contracts", "invoices", "reports", "archive"];

// Store a contract
service.add_node("contracts", "/2024", contract).await?;

// Store an invoice
service.add_node("invoices", "/january", invoice).await?;
```

### Multi-Tenant SaaS

```rust
// Each customer gets isolated workspaces
let ctx = TenantContext::new("customer-123", "production");
let service = NodeService::scoped(storage, ctx);

// Customer's content workspace
service.add_node("content", "/", page).await?;

// Customer's DAM workspace
service.add_node("dam", "/", asset).await?;

// Customer's data workspace
service.add_node("data", "/", record).await?;
```

## Workspace Naming Conventions

**Recommended:**
- Use lowercase, alphanumeric names
- Use hyphens for multi-word names: `customer-data`, `product-catalog`
- Keep names short and descriptive
- Use consistent names across your application

**Examples:**
```rust
// ✅ Good
"content"
"dam"
"customer-data"
"product-catalog"

// ❌ Avoid
"Content_Workspace_2024"  // Too verbose
"ws1"                     // Not descriptive
"My Workspace"            // Contains spaces
```

## Querying Across Workspaces

To query data across multiple workspaces, you need to query each workspace separately:

```rust
async fn get_all_data(service: &NodeService<S>) -> Result<AllData> {
    let content = service.list_all("content").await?;
    let media = service.list_all("dam").await?;
    let customers = service.list_all("customers").await?;

    Ok(AllData { content, media, customers })
}
```

In multi-tenant mode, this automatically respects tenant isolation:

```rust
let acme_ctx = TenantContext::new("acme", "production");
let acme_service = NodeService::scoped(storage.clone(), acme_ctx);

// Only returns Acme's data across all workspaces
let acme_data = get_all_data(&acme_service).await?;
```

## Creating Workspaces

**Important**: Workspaces must be explicitly created before you can add nodes to them.

```rust
use raisin_core::WorkspaceService;
use raisin_models::workspace::Workspace;

let workspace_service = WorkspaceService::new(storage.clone());

// Create workspace first
let workspace = Workspace {
    name: "content".to_string(),
    allowed_node_types: vec!["raisin:Folder".to_string(), "raisin:Page".to_string()],
    allowed_root_node_types: vec!["raisin:Folder".to_string()],
    ..Default::default()
};
workspace_service.put(workspace).await?;

// Now you can add nodes
service.add_node("content", "/", node).await?;  // Works!
```

In multi-tenant mode, each tenant can configure their workspaces independently:

```rust
// Tenant A's workspace configuration
let ctx_a = TenantContext::new("tenant-a", "production");
let workspace_svc_a = WorkspaceService::scoped(storage.clone(), ctx_a.clone());

let workspace = Workspace {
    name: "content".to_string(),
    allowed_node_types: vec!["raisin:Folder".to_string(), "myapp:Article".to_string()],
    allowed_root_node_types: vec!["raisin:Folder".to_string()],
    ..Default::default()
};
workspace_svc_a.put(workspace).await?;

// Tenant B's different workspace configuration
let ctx_b = TenantContext::new("tenant-b", "production");
let workspace_svc_b = WorkspaceService::scoped(storage.clone(), ctx_b.clone());

let workspace = Workspace {
    name: "content".to_string(),
    allowed_node_types: vec!["raisin:Folder".to_string(), "myapp:Product".to_string()],
    allowed_root_node_types: vec!["raisin:Folder".to_string()],
    ..Default::default()
};
workspace_svc_b.put(workspace).await?;

// Each tenant has isolated workspace configurations
```

See [Workspace Configuration](workspace-configuration.md) for detailed setup instructions.

## Key Takeaways

1. **Workspaces organize data types**, not tenants
2. **Workspaces are app-specific** - you define them based on your needs
3. **Workspaces must be explicitly created** with `allowed_node_types` and `allowed_root_node_types`
4. **In multi-tenant mode**, each tenant has their own isolated set of workspaces
5. **Repository = tenant_id + deployment_key** - this is the isolation boundary
6. **Use consistent workspace names** across your application

## Next Steps

- [Workspace Configuration](workspace-configuration.md) - Detailed workspace setup guide
- [Node System](node-system.md) - Understanding NodeTypes and schemas
- [Multi-Tenancy Architecture](multi-tenancy.md) - Understand tenant isolation
- [Building a Multi-Tenant SaaS](../guides/multi-tenant-saas.md) - Complete tutorial
