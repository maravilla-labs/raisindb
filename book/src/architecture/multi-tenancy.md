# Multi-Tenancy Architecture

Understanding how RaisinDB provides tenant isolation and multi-tenancy support.

## Overview

RaisinDB supports multi-tenancy through a combination of:

1. **Tenant Context**: Identifies the tenant and deployment environment
2. **Scoped Services**: Automatically apply tenant context to all operations
3. **Storage Isolation**: Physical or logical data separation
4. **Pluggable Resolution**: Custom tenant extraction logic

## Tenant Context

The `TenantContext` is a simple struct containing:

```rust
pub struct TenantContext {
    tenant_id: String,      // Unique tenant identifier
    deployment: String,     // Environment (production, preview, etc.)
}
```

### Storage Prefix

Each tenant context generates a storage prefix:

```
/{tenant_id}/{deployment}/
```

Examples:

- `/acme/production/` - Acme Corp's production data
- `/acme/preview/` - Acme Corp's preview environment
- `/techco/production/` - TechCo's production data

## Isolation Modes

RaisinDB supports three isolation modes:

### 1. Single Tenant (Default)

```rust
let storage = Arc::new(raisin_rocksdb::open_db("./data")?);
let service = NodeService::new(storage);

// No tenant prefix - keys stored directly
// nodes:workspace:id
```

**Use When:**
- Building a simple application
- Single organization/user
- Embedded usage

**Pros:**
- Simple to use
- Maximum performance
- No overhead

**Cons:**
- No isolation
- Can't easily add tenants later

### 2. Shared Database (Recommended)

```rust
let storage = Arc::new(raisin_rocksdb::open_db("./data")?);
let ctx = TenantContext::new("acme", "production");
let service = NodeService::scoped(storage, ctx);

// Tenant prefix applied - keys are isolated
// /acme/production/nodes:workspace:id
```

**Use When:**
- Building a SaaS application
- Need cost-effective multi-tenancy
- Want easy tenant provisioning

**Pros:**
- Cost effective
- Easy to provision new tenants
- Good performance
- Shared resources

**Cons:**
- Logical isolation only
- Tenants share same DB instance

### 3. Dedicated Database (Enterprise)

```rust
// Each tenant gets their own storage instance
let storage = match tier {
    ServiceTier::Enterprise { connection_string, .. } => {
        Arc::new(raisin_rocksdb::open_db(&connection_string)?)
    }
    _ => shared_storage.clone()
};

let service = NodeService::new(storage);
```

**Use When:**
- Enterprise customers
- Strict isolation requirements
- Regulatory compliance
- Custom performance requirements

**Pros:**
- Complete physical isolation
- Independent scaling
- Custom tuning per tenant

**Cons:**
- Higher cost
- More complex management
- Resource overhead

## How It Works

### Storage Key Prefixing

When using scoped services, all storage keys are automatically prefixed:

```rust
// Single-tenant mode
k_nodes("ws1", "node1", None)
// → "nodes:ws1:node1"

// Multi-tenant mode
let ctx = TenantContext::new("acme", "production");
k_nodes("ws1", "node1", Some(&ctx))
// → "/acme/production/nodes:ws1:node1"
```

This ensures:
- ✅ Complete logical isolation
- ✅ Efficient prefix scans
- ✅ No cross-tenant access
- ✅ Simple backup/restore per tenant

### Scoped Services

The `NodeService::scoped()` constructor creates a tenant-isolated service:

```rust
pub fn scoped(storage: Arc<S>, context: TenantContext) -> NodeService<ScopedStorage<S>> {
    let scoped_storage = Arc::new(storage.scoped(context));
    NodeService::new(scoped_storage)
}
```

Benefits:
- Transparent tenant isolation
- No changes to business logic
- Type-safe at compile time
- Zero runtime overhead

## Tenant Resolution

### Resolver Trait

Implement `TenantResolver` to extract tenant information:

```rust
pub trait TenantResolver: Send + Sync {
    fn resolve(&self, input: &str) -> Option<TenantContext>;
}
```

### Built-in Resolvers

#### Subdomain Resolver

```rust
use raisin_context::SubdomainResolver;

let resolver = SubdomainResolver::new("production");
let ctx = resolver.resolve("acme.myapp.com");
// → TenantContext { tenant_id: "acme", deployment: "production" }
```

#### Fixed Resolver

```rust
use raisin_context::FixedTenantResolver;

let resolver = FixedTenantResolver::new("test-tenant", "dev");
let ctx = resolver.resolve("anything");
// → Always returns same context
```

### Custom Resolvers

Implement your own resolution logic:

```rust
pub struct JwtTenantResolver {
    jwt_secret: String,
}

impl TenantResolver for JwtTenantResolver {
    fn resolve(&self, token: &str) -> Option<TenantContext> {
        let claims = decode_jwt(token, &self.jwt_secret)?;

        Some(TenantContext::new(
            &claims.tenant_id,
            &claims.deployment.unwrap_or("production".to_string())
        ))
    }
}
```

## Data Organization

### Logical Structure

```
RocksDB Instance
├── /tenant-1/
│   ├── production/
│   │   ├── nodes:default:node-1
│   │   ├── nodes:default:node-2
│   │   └── path:default:/page
│   └── preview/
│       ├── nodes:default:node-1
│       └── path:default:/page
├── /tenant-2/
│   └── production/
│       ├── nodes:default:node-1
│       └── nodes:default:node-2
```

### Benefits

1. **Efficient Scans**: Prefix queries are fast
2. **Easy Backups**: Backup per-tenant with key prefix
3. **Simple Deletions**: Delete all tenant data by prefix
4. **Deployment Isolation**: Separate preview/production data

## Workspaces vs Tenancy

**Important distinction**: Workspaces and tenant isolation are separate, orthogonal concepts:

- **Workspaces**: Organize different types of data (e.g., "content", "dam", "customers", "contracts")
- **Tenancy**: Isolate data between different customers
- **Repository**: The combination of `tenant_id + deployment_key`

### Workspaces Are App-Specific

Workspaces let you organize data within a repository:

```rust
// Single workspace used across multiple tenants
let acme_ctx = TenantContext::new("acme", "production");
let acme_service = NodeService::scoped(storage.clone(), acme_ctx);

// Acme's content workspace
acme_service.add_node("content", "/", page).await?;
// Acme's DAM workspace
acme_service.add_node("dam", "/", image).await?;

let techco_ctx = TenantContext::new("techco", "production");
let techco_service = NodeService::scoped(storage.clone(), techco_ctx);

// TechCo's content workspace (isolated from Acme's)
techco_service.add_node("content", "/", page).await?;
// TechCo's DAM workspace (isolated from Acme's)
techco_service.add_node("dam", "/", image).await?;
```

Storage structure:
```
RocksDB
├── /acme/production/
│   ├── nodes:content:node-1    # Acme's website pages
│   └── nodes:dam:logo          # Acme's digital assets
├── /techco/production/
│   ├── nodes:content:node-1    # TechCo's website pages
│   └── nodes:dam:banner        # TechCo's digital assets
```

Each tenant has their own isolated "content" and "dam" workspaces.

### Repository = Tenant + Deployment

In multi-tenant mode, a **repository** is created by combining:

```
repository = tenant_id + deployment_key
```

Each repository can contain multiple workspaces. Examples:

| Tenant | Deployment | Repository | Workspaces |
|--------|------------|------------|-----------|
| `acme` | `production` | Acme's production repo | content, dam, customers |
| `acme` | `preview` | Acme's preview repo | content, dam, customers |
| `techco` | `production` | TechCo's production repo | content, products, orders |

Each customer can have multiple repositories (production, staging, etc.), and each repository can have multiple workspaces.

### Project-Based Example

```rust
// Project A - staging repository
let ctx = TenantContext::new("projecta", "staging");
let service = NodeService::scoped(storage.clone(), ctx);
service.add_node("content", "/", node).await?;
service.add_node("contracts", "/", contract).await?;

// Project A - production repository
let ctx = TenantContext::new("projecta", "production");
let service = NodeService::scoped(storage.clone(), ctx);
service.add_node("content", "/", node).await?;
service.add_node("contracts", "/", contract).await?;
```

See the [Workspaces](workspaces.md) guide for more details.

## Deployment Environments

Each tenant can have multiple deployments:

```rust
// Production environment
let prod_ctx = TenantContext::new("acme", "production");
let prod_service = NodeService::scoped(storage.clone(), prod_ctx);

// Preview environment
let preview_ctx = TenantContext::new("acme", "preview");
let preview_service = NodeService::scoped(storage.clone(), preview_ctx);

// Changes to preview don't affect production
```

Common patterns:

- **production**: Live customer data
- **preview**: Next version preview
- **staging**: Testing before production
- **dev**: Development environment
- **feature-X**: Feature branch environment

## Cross-Tenant Operations

### Admin Operations

Sometimes you need to operate across tenants:

```rust
async fn list_all_tenants(storage: Arc<RocksDBStorage>) -> Vec<String> {
    // Iterate all keys and extract unique tenant IDs
    // This requires direct storage access
    extract_tenant_ids_from_keys(storage).await
}

async fn migrate_all_tenants(storage: Arc<RocksDBStorage>) {
    for tenant_id in list_all_tenants(storage.clone()).await {
        let ctx = TenantContext::new(&tenant_id, "production");
        let service = NodeService::scoped(storage.clone(), ctx);

        // Perform migration for this tenant
        migrate_tenant(service).await;
    }
}
```

### Reporting

Aggregate data across tenants:

```rust
async fn get_platform_stats(storage: Arc<RocksDBStorage>) -> PlatformStats {
    let mut stats = PlatformStats::default();

    for tenant_id in list_all_tenants(storage.clone()).await {
        let ctx = TenantContext::new(&tenant_id, "production");
        let service = NodeService::scoped(storage.clone(), ctx);

        let node_count = service.list_all("default").await?.len();
        stats.total_nodes += node_count;
        stats.tenant_count += 1;
    }

    stats
}
```

## Security Best Practices

### 1. Always Validate Context

```rust
fn validate_tenant_access(
    user: &User,
    context: &TenantContext,
) -> Result<(), Error> {
    if user.tenant_id != context.tenant_id() {
        return Err(Error::Forbidden);
    }
    Ok(())
}
```

### 2. Use Type System

```rust
// Good - tenant context is part of the service type
let service: NodeService<ScopedStorage<RocksDBStorage>> =
    NodeService::scoped(storage, ctx);

// Service can only access tenant's data
```

### 3. Audit Tenant Access

```rust
async fn audit_tenant_access(
    user_id: &str,
    tenant_id: &str,
    operation: &str,
) {
    log::info!(
        "User {} accessed tenant {} for operation {}",
        user_id, tenant_id, operation
    );

    // Store in audit log
    audit_log.record(user_id, tenant_id, operation).await;
}
```

## Performance Considerations

### Prefix Queries

RocksDB prefix queries are efficient:

```rust
// Fast - single prefix scan
let nodes = service.list_all("workspace").await?;
// Scans only: /tenant-1/production/nodes:workspace:*
```

### Shared Resources

Monitor resource usage per tenant:

```rust
struct TenantMetrics {
    storage_size: u64,
    request_count: u64,
    node_count: u64,
}

async fn get_tenant_metrics(tenant_id: &str) -> TenantMetrics {
    // Calculate from storage and request logs
    calculate_metrics(tenant_id).await
}
```

## Migration Strategies

### Adding Multi-Tenancy to Existing App

```rust
async fn migrate_to_multi_tenant(
    old_storage: Arc<RocksDBStorage>,
    new_storage: Arc<RocksDBStorage>,
    tenant_id: &str,
) -> Result<()> {
    // Read from old single-tenant storage
    let old_service = NodeService::new(old_storage);
    let nodes = old_service.list_all("default").await?;

    // Write to new multi-tenant storage
    let ctx = TenantContext::new(tenant_id, "production");
    let new_service = NodeService::scoped(new_storage, ctx);

    for node in nodes {
        new_service.put("default", node).await?;
    }

    Ok(())
}
```

## Next Steps

- [Building a SaaS](../guides/multi-tenant-saas.md) - Complete tutorial
- [Rate Limiting](rate-limiting.md) - Per-tenant rate limits
- [Service Tiers](../guides/tier-systems.md) - Tier-based features
