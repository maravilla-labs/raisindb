# Context System API

Multi-tenancy context types and traits.

## TenantContext

Identifies a tenant and deployment environment.

```rust
pub struct TenantContext {
    tenant_id: String,
    deployment: String,
}
```

### Methods

#### `new(tenant_id: impl Into<String>, deployment: impl Into<String>) -> Self`

Create a new tenant context.

```rust
let ctx = TenantContext::new("customer-123", "production");
```

#### `tenant_id(&self) -> &str`

Get the tenant ID.

#### `deployment(&self) -> &str`

Get the deployment name.

#### `storage_prefix(&self) -> String`

Get the storage key prefix: `/{tenant_id}/{deployment}`

## TenantResolver Trait

Extract tenant from request information.

```rust
pub trait TenantResolver: Send + Sync {
    fn resolve(&self, input: &str) -> Option<TenantContext>;
}
```

### Implementations

#### SubdomainResolver

Extract tenant from subdomain.

```rust
let resolver = SubdomainResolver::new("production");
let ctx = resolver.resolve("acme.myapp.com");
// → TenantContext { tenant_id: "acme", deployment: "production" }
```

#### FixedTenantResolver

Always returns same context (for testing).

```rust
let resolver = FixedTenantResolver::new("test-tenant", "dev");
```

## IsolationMode

Defines how tenant data is isolated.

```rust
pub enum IsolationMode {
    Single,                          // No isolation
    Shared(TenantContext),           // Logical isolation
    Dedicated {                      // Physical isolation
        context: TenantContext,
        connection_string: String,
    },
}
```

## ServiceTier

Service tier definitions.

```rust
pub enum ServiceTier {
    Free {
        max_nodes: usize,
        max_requests_per_minute: usize,
    },
    Professional {
        max_nodes: usize,
        max_requests_per_minute: usize,
    },
    Enterprise {
        dedicated_db: bool,
        max_requests_per_minute: usize,
        custom_features: Vec<String>,
    },
}
```

### Methods

#### `max_nodes(&self) -> Option<usize>`

Get maximum nodes allowed.

#### `rate_limit(&self) -> usize`

Get rate limit in requests per minute.

#### `has_dedicated_db(&self) -> bool`

Check if tier has dedicated database.

## TierProvider Trait

Determine and enforce service tiers.

```rust
pub trait TierProvider: Send + Sync {
    async fn get_tier(&self, tenant_id: &str) -> ServiceTier;
    async fn check_limits(&self, tenant_id: &str, operation: &Operation) -> Result<(), String>;
    async fn record_usage(&self, tenant_id: &str, operation: &Operation);
}
```
