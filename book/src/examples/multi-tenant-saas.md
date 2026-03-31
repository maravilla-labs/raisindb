# Multi-Tenant SaaS Example

This example demonstrates a complete multi-tenant SaaS application.

## Location

`examples/multi-tenant-saas/`

## Running

```bash
cargo run --example multi-tenant-saas
```

## Features

- Subdomain-based tenant resolution
- Service tier enforcement
- Rate limiting per tenant
- Complete HTTP API

## Testing

```bash
# Create node for tenant "acme"
curl -H "Host: acme.localhost:3000" \
     -H "Content-Type: application/json" \
     -X POST http://localhost:3000/api/nodes \
     -d '{"name":"my-page","node_type":"raisin:Folder"}'

# List nodes for tenant "acme"
curl -H "Host: acme.localhost:3000" \
     http://localhost:3000/api/nodes

# Different tenant - isolated data
curl -H "Host: techco.localhost:3000" \
     http://localhost:3000/api/nodes
```

## Code Walkthrough

### Tenant Resolution

```rust
use raisin_context::TenantContext;

fn extract_tenant_from_host(host: &str) -> Option<TenantContext> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    let subdomain = parts[0];

    // Skip non-tenant subdomains
    if subdomain == "www" || subdomain == "api" {
        return None;
    }

    Some(TenantContext::new(subdomain, "production"))
}
```

### Scoped Service Creation

```rust
async fn create_node(
    Host(host): Host,
    State(state): State<AppState>,
    Json(node): Json<Node>,
) -> Result<Json<Node>, StatusCode> {
    // 1. Extract tenant
    let tenant_ctx = extract_tenant_from_host(&host)
        .ok_or(StatusCode::BAD_REQUEST)?;

    // 2. Create scoped service via RaisinConnection
    let conn = RaisinConnection::with_storage(state.storage.clone());
    let node_service = conn.tenant(tenant_ctx.tenant_id())
        .repository("app")
        .workspace("default")
        .nodes();

    // 3. Perform operation (automatically tenant-isolated)
    let created = node_service
        .add_node("/", node)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(created))
}
```

### Tier Provider

```rust
impl TierProvider for SimpleTierProvider {
    async fn get_tier(&self, tenant_id: &str) -> ServiceTier {
        match tenant_id {
            tid if tid.starts_with("enterprise-") => ServiceTier::Enterprise {
                dedicated_db: true,
                max_requests_per_minute: 10_000,
                custom_features: vec![],
            },
            tid if tid.starts_with("pro-") => ServiceTier::Professional {
                max_nodes: 100_000,
                max_requests_per_minute: 1_000,
            },
            _ => ServiceTier::Free {
                max_nodes: 1_000,
                max_requests_per_minute: 100,
            },
        }
    }
}
```

## Architecture

```text
HTTP Request (subdomain: acme.localhost:3000)
          ↓
Extract Tenant Context (acme, production)
          ↓
Check Rate Limits (100 req/min for free tier)
          ↓
Check Tier Limits (1000 nodes max)
          ↓
Create Scoped Service (isolated to acme/production)
          ↓
Perform Operation (all data scoped to acme)
          ↓
Record Usage (for billing/analytics)
          ↓
Return Response
```

## Next Steps

After running this example, try:

1. Modifying the tier provider to use a real database
2. Implementing JWT-based tenant resolution
3. Adding custom rate limiting rules
4. Integrating with your billing system

See the [Multi-Tenant SaaS Guide](../guides/multi-tenant-saas.md) for more details.
