# Multi-Tenant SaaS Example

This example demonstrates how to build a multi-tenant SaaS application using RaisinDB.

## Features

- **Subdomain-based tenant isolation**: Each tenant is identified by their subdomain
- **Automatic data isolation**: All tenant data is automatically scoped and isolated
- **Service tiers**: Free, Professional, and Enterprise tiers with different limits
- **Rate limiting**: Per-tenant rate limits based on their tier
- **RocksDB storage**: Persistent, high-performance storage with tenant prefixing

## Architecture

```
┌─────────────────────────────────────────────────┐
│  HTTP Request: acme.yourapp.com/api/nodes       │
└─────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────┐
│  Tenant Resolver Middleware                     │
│  - Extract "acme" from subdomain                │
│  - Create TenantContext("acme", "production")   │
└─────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────┐
│  Tier Provider                                  │
│  - Check tenant tier (Free/Pro/Enterprise)      │
│  - Enforce rate limits                          │
└─────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────┐
│  Scoped NodeService                             │
│  - NodeService::scoped(storage, tenant_ctx)     │
│  - All operations automatically tenant-scoped   │
└─────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────┐
│  RocksDB Storage                                │
│  - Keys: /acme/production/nodes:default:id      │
│  - Complete data isolation per tenant           │
└─────────────────────────────────────────────────┘
```

## Running

```bash
# From the repository root
cargo run --example multi-tenant-saas
```

## Testing

### Using curl with Host header:

```bash
# Create a node for tenant "acme"
curl -H "Host: acme.localhost:3000" \
     -H "Content-Type: application/json" \
     -X POST http://localhost:3000/api/nodes \
     -d '{
       "name": "my-page",
       "node_type": "raisin:Folder",
       "properties": {}
     }'

# List all nodes for tenant "acme"
curl -H "Host: acme.localhost:3000" \
     http://localhost:3000/api/nodes

# Create a node for tenant "enterprise-acme" (different tier)
curl -H "Host: enterprise-acme.localhost:3000" \
     -H "Content-Type: application/json" \
     -X POST http://localhost:3000/api/nodes \
     -d '{
       "name": "enterprise-page",
       "node_type": "raisin:Folder",
       "properties": {}
     }'

# Verify tenant isolation - should only see their own nodes
curl -H "Host: acme.localhost:3000" \
     http://localhost:3000/api/nodes
```

### Service Tiers

The example includes three tiers:

1. **Free Tier** (default)
   - 1,000 max nodes
   - 100 requests/minute
   - Example: `acme.localhost:3000`

2. **Professional Tier** (prefix: `pro-`)
   - 100,000 max nodes
   - 1,000 requests/minute
   - Example: `pro-company.localhost:3000`

3. **Enterprise Tier** (specific tenants)
   - Unlimited nodes
   - 10,000 requests/minute
   - Dedicated database support
   - Example: `enterprise-acme.localhost:3000`

## Embedding in Your SaaS

To use RaisinDB in your own multi-tenant application:

1. **Implement TenantResolver**: Extract tenant from your auth system
2. **Implement TierProvider**: Connect to your billing database
3. **Use Scoped Services**: Create tenant-scoped services per request
4. **Add Rate Limiting**: Use `RocksRateLimiter` for enforcement

See the [embedding guide](../../docs/embedding/multi-tenant-setup.md) for details.

## Data Storage

Data is stored in `./data/multi-tenant/` with the following key structure:

```
/acme/production/nodes:default:node-id-123
/acme/production/path:default:/my-page
/pro-company/production/nodes:default:node-id-456
/enterprise-acme/production/nodes:default:node-id-789
```

This ensures complete isolation between tenants at the storage level.
