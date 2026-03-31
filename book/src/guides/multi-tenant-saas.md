# Building a Multi-Tenant SaaS

This guide shows you how to build a complete multi-tenant SaaS application with RaisinDB.

## Overview

In a multi-tenant architecture, you serve multiple customers (tenants) from a single application instance while keeping their data completely isolated.

### Benefits

- **Cost Efficiency**: Share infrastructure across all tenants
- **Easy Updates**: Deploy once, update for everyone
- **Scalability**: Add new tenants without new deployments
- **Resource Sharing**: Better resource utilization

### Isolation Strategy

RaisinDB supports multiple isolation strategies:

1. **Shared Database with Logical Isolation** (Recommended)
   - All tenants share the same RocksDB instance
   - Keys are prefixed with tenant ID and deployment
   - Excellent performance and resource efficiency

2. **Dedicated Database** (Enterprise)
   - Each tenant gets their own RocksDB instance
   - Complete physical isolation
   - Higher resource usage

## Architecture

```
HTTP Request → Tenant Resolver → Tier Check → Scoped Service → Isolated Storage
```

### Key Concepts

- **Tenant**: A customer using your SaaS (e.g., "acme", "techco")
- **Deployment**: Environment within a tenant (e.g., "production", "staging", "preview")
- **Repository**: Combination of tenant_id + deployment (e.g., "acme/production")
- **Workspace**: App-specific data category (e.g., "content", "dam", "customers")

Each tenant gets their own repository, and within each repository, you can have multiple workspaces to organize different types of data.

## Step-by-Step Implementation

### 1. Initialize Storage and Setup

Before handling requests, set up your storage and define schemas:

```rust
use raisin_core::{RaisinConnection, WorkspaceService};
use raisin_rocksdb::RocksDBStorage;
use raisin_models::{
    nodes::types::node_type::NodeType,
    nodes::properties::schema::PropertyValueSchema,
    nodes::properties::schema::PropertyType,
    workspace::Workspace,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize storage
    let storage = Arc::new(RocksDBStorage::new("./data")?);

    // Setup NodeTypes and Workspaces (see sections below)
    setup_global_nodetypes(&storage).await?;

    // Continue with application setup...
    Ok(())
}
```

### 2. Define NodeTypes

Create custom NodeTypes for your application. In multi-tenant mode, you typically define NodeTypes once globally or per-tenant:

#### Option A: Global NodeTypes (Recommended)

Define NodeTypes that all tenants will use:

```rust
async fn setup_global_nodetypes(storage: &Arc<RocksDBStorage>) -> anyhow::Result<()> {
    // NodeTypes are managed via the storage's node_types() repository
    // or registered through the HTTP API / package system

    // Article NodeType
    let article_type = NodeType {
        name: "Article".to_string(),
        description: Some("A blog article or news post".to_string()),
        properties: Some(vec![
            PropertyValueSchema {
                name: Some("title".to_string()),
                property_type: PropertyType::String,
                required: Some(true),
                constraints: Some({
                    let mut c = HashMap::new();
                    c.insert("minLength".to_string(), PropertyValue::Number(1.0));
                    c.insert("maxLength".to_string(), PropertyValue::Number(200.0));
                    c
                }),
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("content".to_string()),
                property_type: PropertyType::String,
                required: Some(true),
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("author".to_string()),
                property_type: PropertyType::Reference,
                meta: Some({
                    let mut m = HashMap::new();
                    m.insert("allowedTypes".to_string(),
                        PropertyValue::Array(vec![PropertyValue::String("User".to_string())]));
                    m.insert("workspace".to_string(),
                        PropertyValue::String("users".to_string()));
                    m
                }),
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("publishedAt".to_string()),
                property_type: PropertyType::Date,
                ..Default::default()
            },
        ]),
        versionable: Some(true),
        publishable: Some(true),
        auditable: Some(true),
        ..Default::default()
    };
    // Register the NodeType via the storage repository
    storage.node_types().put("global", "default", article_type).await?;

    // Product NodeType
    let product_type = NodeType {
        name: "Product".to_string(),
        description: Some("E-commerce product".to_string()),
        properties: Some(vec![
            PropertyValueSchema {
                name: Some("name".to_string()),
                property_type: PropertyType::String,
                required: Some(true),
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("price".to_string()),
                property_type: PropertyType::Number,
                required: Some(true),
                constraints: Some({
                    let mut c = HashMap::new();
                    c.insert("minimum".to_string(), PropertyValue::Number(0.0));
                    c
                }),
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("description".to_string()),
                property_type: PropertyType::String,
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("images".to_string()),
                property_type: PropertyType::Array,
                items: Some(Box::new(PropertyValueSchema {
                    property_type: PropertyType::Resource,
                    ..Default::default()
                })),
                ..Default::default()
            },
        ]),
        publishable: Some(true),
        ..Default::default()
    };
    storage.node_types().put("global", "default", product_type).await?;

    println!("✓ Created global NodeTypes");
    Ok(())
}
```

#### Option B: Per-Tenant NodeTypes

Allow each tenant to define their own NodeTypes:

```rust
async fn setup_tenant_nodetypes(
    storage: Arc<RocksDBStorage>,
    tenant_id: &str,
) -> anyhow::Result<()> {
    // Tenant-specific Article type
    let article_type = NodeType {
        name: "Article".to_string(),
        properties: Some(vec![
            PropertyValueSchema {
                name: Some("title".to_string()),
                property_type: PropertyType::String,
                required: Some(true),
                ..Default::default()
            },
            // Tenant-specific fields...
        ]),
        ..Default::default()
    };

    // Store NodeType scoped to this tenant's repository
    storage.node_types().put(tenant_id, "production", article_type).await?;
    Ok(())
}
```

### 3. Create Workspaces

Each tenant needs workspaces configured before they can create nodes:

```rust
async fn setup_tenant_workspaces(
    storage: Arc<RocksDBStorage>,
    tenant_id: &str,
) -> anyhow::Result<()> {
    let workspace_service = WorkspaceService::new(storage.clone());
    let repo_id = "production";

    // Content workspace
    let mut content_workspace = Workspace::new("content".to_string());
    content_workspace.description = Some("Website content and blog posts".to_string());
    content_workspace.allowed_node_types = vec![
        "raisin:Folder".to_string(),
        "myapp:Article".to_string(),
    ];
    content_workspace.allowed_root_node_types = vec![
        "raisin:Folder".to_string(),
    ];
    workspace_service.put(tenant_id, repo_id, content_workspace).await?;

    // DAM (Digital Asset Management) workspace
    let mut dam_workspace = Workspace::new("dam".to_string());
    dam_workspace.description = Some("Digital assets - images, videos, files".to_string());
    dam_workspace.allowed_node_types = vec![
        "raisin:Folder".to_string(),
        "raisin:Asset".to_string(),
    ];
    dam_workspace.allowed_root_node_types = vec![
        "raisin:Folder".to_string(),
    ];
    workspace_service.put(tenant_id, repo_id, dam_workspace).await?;

    // Products workspace
    let mut products_workspace = Workspace::new("products".to_string());
    products_workspace.description = Some("E-commerce product catalog".to_string());
    products_workspace.allowed_node_types = vec![
        "raisin:Folder".to_string(),
        "myapp:Product".to_string(),
    ];
    products_workspace.allowed_root_node_types = vec![
        "raisin:Folder".to_string(),
    ];
    workspace_service.put(tenant_id, repo_id, products_workspace).await?;

    // Users workspace
    let mut users_workspace = Workspace::new("users".to_string());
    users_workspace.description = Some("User accounts and profiles".to_string());
    users_workspace.allowed_node_types = vec![
        "raisin:User".to_string(),
    ];
    users_workspace.allowed_root_node_types = vec![
        "raisin:User".to_string(),
    ];
    workspace_service.put(tenant_id, repo_id, users_workspace).await?;

    println!("Created workspaces for tenant: {}", tenant_id);
    Ok(())
}
```

### 4. Tenant Onboarding Flow

When a new tenant signs up, initialize their environment:

```rust
async fn onboard_new_tenant(
    storage: Arc<RocksDBStorage>,
    tenant_id: &str,
    tier: ServiceTier,
) -> anyhow::Result<()> {
    println!("Onboarding tenant: {}", tenant_id);

    // 1. Setup workspaces
    setup_tenant_workspaces(storage.clone(), tenant_id).await?;

    // 2. Create initial structure
    create_initial_structure(storage.clone(), tenant_id).await?;

    // 3. Record tenant in billing system
    record_tenant_in_billing(tenant_id, tier).await?;

    println!("✓ Tenant onboarded: {}", tenant_id);
    Ok(())
}

async fn create_initial_structure(
    storage: Arc<RocksDBStorage>,
    tenant_id: &str,
) -> anyhow::Result<()> {
    let connection = RaisinConnection::with_storage(storage);

    // Create root folders in each workspace
    for workspace in ["content", "dam", "products"] {
        let service = connection
            .tenant(tenant_id)
            .repository("production")
            .workspace(workspace)
            .nodes();

        let mut properties = HashMap::new();
        properties.insert(
            "title".to_string(),
            PropertyValue::String(format!("{} Root", workspace)),
        );

        let root_folder = Node {
            name: "root".to_string(),
            node_type: "raisin:Folder".to_string(),
            properties,
            ..Default::default()
        };
        service.add_node("/", root_folder).await?;
    }

    Ok(())
}
```

### 5. Setup Dependencies

```toml
[dependencies]
raisin-core = { path = "path/to/raisin-core" }
raisin-rocksdb = { path = "path/to/raisin-rocksdb" }
raisin-models = { path = "path/to/raisin-models" }
raisin-context = { path = "path/to/raisin-context" }
raisin-ratelimit = { path = "path/to/raisin-ratelimit" }
axum = "0.7"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

### 6. Implement Tenant Resolver

Extract tenant information from requests:

RaisinDB includes a built-in `SubdomainResolver`, or you can implement the `TenantResolver` trait:

```rust
use raisin_context::{TenantContext, TenantResolver};

// The built-in SubdomainResolver handles subdomain extraction:
// "acme.myapp.com" -> tenant: "acme", deployment: "production"
use raisin_context::SubdomainResolver;
let resolver = SubdomainResolver::new("production");

// Or implement your own:
pub struct HeaderTenantResolver;

impl TenantResolver for HeaderTenantResolver {
    fn resolve(&self, input: &str) -> Option<TenantContext> {
        // The `input` parameter receives whatever string you pass
        // (e.g., hostname, header value, JWT token)
        if input.is_empty() {
            return None;
        }
        Some(TenantContext::new(input, "production"))
    }
}
```

Alternative approaches:

```rust
// JWT-based resolution
pub struct JwtTenantResolver;

impl TenantResolver for JwtTenantResolver {
    fn resolve(&self, token: &str) -> Option<TenantContext> {
        // Decode JWT and extract tenant_id claim
        let claims = decode_jwt(token)?;
        Some(TenantContext::new(&claims.tenant_id, "production"))
    }
}
```

### 7. Implement Tier Provider

Connect to your billing system:

```rust
use raisin_context::{ServiceTier, TierProvider, Operation};

pub struct DatabaseTierProvider {
    pool: sqlx::PgPool,
}

impl TierProvider for DatabaseTierProvider {
    async fn get_tier(&self, tenant_id: &str) -> ServiceTier {
        // Query your billing database
        let tier = sqlx::query!(
            "SELECT tier FROM subscriptions WHERE tenant_id = $1",
            tenant_id
        )
        .fetch_one(&self.pool)
        .await
        .ok();

        match tier.as_deref().map(|t| t.tier.as_str()) {
            Some("enterprise") => ServiceTier::Enterprise {
                dedicated_db: true,
                max_requests_per_minute: 10_000,
                custom_features: vec![],
            },
            Some("professional") => ServiceTier::Professional {
                max_nodes: 100_000,
                max_requests_per_minute: 1_000,
            },
            _ => ServiceTier::Free {
                max_nodes: 1_000,
                max_requests_per_minute: 100,
            },
        }
    }

    async fn check_limits(
        &self,
        tenant_id: &str,
        operation: &Operation,
    ) -> Result<(), String> {
        let tier = self.get_tier(tenant_id).await;

        match operation {
            Operation::CreateNode => {
                // Check node count against tier limit
                let count = self.count_nodes(tenant_id).await;
                if let Some(max) = tier.max_nodes() {
                    if count >= max {
                        return Err(format!(
                            "Node limit reached: {}/{}. Upgrade to increase limit.",
                            count, max
                        ));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn record_usage(&self, tenant_id: &str, operation: &Operation) {
        // Record to analytics/billing database
        sqlx::query!(
            "INSERT INTO usage_events (tenant_id, operation, created_at) VALUES ($1, $2, NOW())",
            tenant_id,
            format!("{:?}", operation)
        )
        .execute(&self.pool)
        .await
        .ok();
    }
}
```

### 8. Setup Rate Limiting

```rust
use raisin_ratelimit::RocksRateLimiter;
use raisin_context::{RateLimiter, RateLimitInfo};
use std::time::Duration;

pub struct RateLimitMiddleware {
    limiter: RocksRateLimiter,
    tier_provider: Arc<dyn TierProvider>,
}

impl RateLimitMiddleware {
    pub async fn check(
        &self,
        tenant_id: &str,
    ) -> Result<(), StatusCode> {
        let tier = self.tier_provider.get_tier(tenant_id).await;
        let limit = tier.rate_limit();

        let info = self.limiter
            .check_rate(tenant_id, limit, Duration::from_secs(60))
            .await;

        if !info.allowed {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }

        Ok(())
    }
}
```

### 9. Create Application State

```rust
#[derive(Clone)]
pub struct AppState {
    storage: Arc<RocksDBStorage>,
    tier_provider: Arc<dyn TierProvider>,
    rate_limiter: Arc<RocksRateLimiter>,
    tenant_resolver: Arc<dyn TenantResolver>,
}
```

### 10. Build Request Handlers

**Important**: Before handling requests, ensure workspaces are created for the tenant (see section 3). The handlers below assume workspaces already exist.

```rust
use axum::{extract::{Host, State}, Json};

async fn create_node(
    Host(host): Host,
    State(state): State<AppState>,
    Json(node): Json<Node>,
) -> Result<Json<Node>, StatusCode> {
    // 1. Resolve tenant
    let tenant_ctx = state.tenant_resolver
        .resolve(&host)
        .ok_or(StatusCode::BAD_REQUEST)?;

    // 2. Check rate limit
    check_rate_limit(&state, tenant_ctx.tenant_id()).await?;

    // 3. Check tier limits
    state.tier_provider
        .check_limits(tenant_ctx.tenant_id(), &Operation::CreateNode)
        .await
        .map_err(|_| StatusCode::PAYMENT_REQUIRED)?;

    // 4. Create scoped service via connection API
    let workspace = extract_workspace(&headers).unwrap_or("content".to_string());
    let connection = RaisinConnection::with_storage(state.storage.clone());
    let service = connection
        .tenant(tenant_ctx.tenant_id())
        .repository(tenant_ctx.deployment())
        .workspace(&workspace)
        .nodes();

    // 5. Perform operation
    let created = service
        .add_node("/", node)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 6. Record usage
    state.tier_provider
        .record_usage(tenant_ctx.tenant_id(), &Operation::CreateNode)
        .await;

    Ok(Json(created))
}

// Helper to extract workspace from request
fn extract_workspace(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-workspace")
        .and_then(|h| h.to_str().ok())
        .map(String::from)
}
```

### 11. Setup Router

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize components
    let storage = Arc::new(RocksDBStorage::new("./data")?);
    let tier_provider = Arc::new(DatabaseTierProvider::new().await?);
    let rate_limiter = Arc::new(RocksRateLimiter::open("./rate-limits")?);
    let tenant_resolver = Arc::new(SubdomainResolver::new("production"));

    let state = AppState {
        storage,
        tier_provider,
        rate_limiter,
        tenant_resolver,
    };

    let app = Router::new()
        .route("/api/nodes", post(create_node).get(list_nodes))
        .route("/api/nodes/:id", get(get_node))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

## Deployment Environments

Support multiple environments per tenant:

```rust
pub fn get_deployment_from_header(headers: &HeaderMap) -> String {
    headers
        .get("x-deployment")
        .and_then(|h| h.to_str().ok())
        .map(String::from)
        .unwrap_or_else(|| "production".to_string())
}

// Create context with custom deployment
let ctx = TenantContext::new(tenant_id, &deployment);
```

Common patterns:

- **production**: Live customer data
- **preview**: Preview/staging environment
- **development**: Development environment

## Working with Workspaces

Each tenant can use multiple workspaces to organize their data. After workspaces are created (see section 3), you can add nodes to them:

### Querying Across Workspaces

```rust
async fn get_tenant_dashboard(
    storage: Arc<RocksDBStorage>,
    tenant_id: &str,
) -> Result<DashboardData> {
    let connection = RaisinConnection::with_storage(storage);
    let tenant = connection.tenant(tenant_id).repository("production");

    // Query different workspaces
    let pages = tenant.workspace("content").nodes().list_all().await?;
    let assets = tenant.workspace("dam").nodes().list_all().await?;
    let customers = tenant.workspace("customers").nodes().list_all().await?;

    Ok(DashboardData {
        page_count: pages.len(),
        asset_count: assets.len(),
        customer_count: customers.len(),
    })
}
```

### Workspace-Based Routing

```rust
// Route pattern: /api/:workspace/nodes
async fn workspace_handler(
    Host(host): Host,
    Path(workspace): Path<String>,
    State(state): State<AppState>,
    Json(node): Json<Node>,
) -> Result<Json<Node>, StatusCode> {
    // Resolve tenant
    let ctx = state.tenant_resolver
        .resolve(&host)
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Validate workspace name
    let allowed_workspaces = ["content", "dam", "customers", "products"];
    if !allowed_workspaces.contains(&workspace.as_str()) {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Create scoped service via connection API
    let connection = RaisinConnection::with_storage(state.storage.clone());
    let service = connection
        .tenant(ctx.tenant_id())
        .repository(ctx.deployment())
        .workspace(&workspace)
        .nodes();

    // Create node in specified workspace
    let created = service
        .add_node("/", node)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(created))
}

// Setup routes
let app = Router::new()
    .route("/api/:workspace/nodes", post(workspace_handler))
    .with_state(state);
```

## Data Migration

### Migrating Existing Single-Tenant to Multi-Tenant

```rust
async fn migrate_to_multi_tenant(
    storage: Arc<RocksDBStorage>,
    tenant_id: &str,
) -> Result<()> {
    let connection = RaisinConnection::with_storage(storage);

    // 1. Get all nodes from single-tenant "default" storage
    let source = connection
        .tenant("default")
        .repository("default")
        .workspace("default")
        .nodes();
    let nodes = source.list_all().await?;

    // 2. Create service scoped to the target tenant
    let target = connection
        .tenant(tenant_id)
        .repository("production")
        .workspace("default")
        .nodes();

    // 3. Copy nodes to the new tenant scope
    for node in nodes {
        target.add_node("/", node).await?;
    }

    Ok(())
}
```

## Security Considerations

### 1. Tenant Isolation

Always validate tenant context:

```rust
fn validate_tenant_access(
    user_tenant: &str,
    requested_tenant: &str,
) -> Result<(), StatusCode> {
    if user_tenant != requested_tenant {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}
```

### 2. Rate Limiting

Implement aggressive rate limiting:

```rust
// Per-tenant limits
let tenant_limit = tier.rate_limit();

// Global limits (prevent single tenant from DoS)
let global_limit = 100_000;

check_both_limits(tenant_id, tenant_limit, global_limit).await?;
```

### 3. Resource Quotas

Enforce strict quotas:

```rust
async fn enforce_quotas(
    tenant_id: &str,
    tier: &ServiceTier,
) -> Result<(), String> {
    let usage = get_current_usage(tenant_id).await;

    if let Some(max_nodes) = tier.max_nodes() {
        if usage.node_count >= max_nodes {
            return Err("Node quota exceeded".to_string());
        }
    }

    Ok(())
}
```

## Monitoring

### Track Per-Tenant Metrics

```rust
async fn record_metrics(
    tenant_id: &str,
    operation: &Operation,
    duration: Duration,
) {
    metrics::histogram!(
        "operation_duration",
        duration.as_secs_f64(),
        "tenant" => tenant_id,
        "operation" => format!("{:?}", operation)
    );
}
```

### Alert on Anomalies

```rust
async fn check_anomalies(tenant_id: &str) {
    let recent_usage = get_recent_usage(tenant_id, Duration::from_hours(1)).await;

    if recent_usage.request_count > 10_000 {
        alert_ops_team(tenant_id, "Unusual traffic spike detected");
    }
}
```

## Complete Example

See the [multi-tenant-saas example](../../examples/multi-tenant-saas/) for a complete working implementation.

## Next Steps

- [Service Tiers](tier-systems.md) - Implement tier-based features
- [Rate Limiting](../architecture/rate-limiting.md) - Deep dive into rate limiting
- [Custom Storage](custom-storage.md) - Build custom storage backends
