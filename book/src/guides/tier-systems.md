# Service Tier Systems

Service tiers allow you to provide different levels of service to different tenants based on their subscription or plan.

## Tier Definition

RaisinDB includes a `ServiceTier` enum with common tiers:

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

## TierProvider Trait

Implement this trait to connect to your billing system:

```rust
use raisin_context::{TierProvider, ServiceTier, Operation};

pub struct MyTierProvider {
    database: sqlx::PgPool,
}

impl TierProvider for MyTierProvider {
    async fn get_tier(&self, tenant_id: &str) -> ServiceTier {
        // Query your billing database
        let subscription = sqlx::query!(
            "SELECT plan FROM subscriptions WHERE tenant_id = $1",
            tenant_id
        )
        .fetch_one(&self.database)
        .await
        .ok();
        
        match subscription.map(|s| s.plan.as_str()) {
            Some("enterprise") => ServiceTier::Enterprise {
                dedicated_db: true,
                max_requests_per_minute: 10_000,
                custom_features: vec!["advanced-analytics".to_string()],
            },
            Some("pro") => ServiceTier::Professional {
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
                if let Some(max) = tier.max_nodes() {
                    let current = self.count_nodes(tenant_id).await;
                    if current >= max {
                        return Err(format!(
                            "Node limit reached: {}/{}. Please upgrade your plan.",
                            current, max
                        ));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    
    async fn record_usage(&self, tenant_id: &str, operation: &Operation) {
        // Log to analytics/billing
        sqlx::query!(
            "INSERT INTO usage_log (tenant_id, operation) VALUES ($1, $2)",
            tenant_id,
            format!("{:?}", operation)
        )
        .execute(&self.database)
        .await
        .ok();
    }
}
```

## Integration Example

```rust
use axum::{extract::State, Json};

#[derive(Clone)]
struct AppState {
    storage: Arc<RocksDBStorage>,
    tier_provider: Arc<MyTierProvider>,
}

async fn create_node(
    State(state): State<AppState>,
    Extension(ctx): Extension<TenantContext>,
    Json(node): Json<Node>,
) -> Result<Json<Node>, StatusCode> {
    // Check tier limits
    state.tier_provider
        .check_limits(ctx.tenant_id(), &Operation::CreateNode)
        .await
        .map_err(|_| StatusCode::PAYMENT_REQUIRED)?;
    
    // Create scoped service via connection API
    let connection = RaisinConnection::with_storage(state.storage.clone());
    let service = connection
        .tenant(ctx.tenant_id())
        .repository(ctx.deployment())
        .workspace("default")
        .nodes();

    // Perform operation
    let created = service
        .add_node("/", node)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    // Record usage
    state.tier_provider
        .record_usage(ctx.tenant_id(), &Operation::CreateNode)
        .await;
    
    Ok(Json(created))
}
```

## Upgrade Prompts

Show upgrade prompts when limits are approached:

```rust
async fn check_usage_and_prompt(
    tier_provider: &impl TierProvider,
    tenant_id: &str,
) -> UsageInfo {
    let tier = tier_provider.get_tier(tenant_id).await;
    let usage = get_current_usage(tenant_id).await;
    
    let usage_pct = if let Some(max) = tier.max_nodes() {
        (usage.node_count as f64 / max as f64) * 100.0
    } else {
        0.0
    };
    
    UsageInfo {
        tier,
        usage_percent: usage_pct,
        should_upgrade: usage_pct > 80.0,
    }
}
```

## Feature Flags

Enable features based on tier:

```rust
impl ServiceTier {
    pub fn has_feature(&self, feature: &str) -> bool {
        match self {
            ServiceTier::Enterprise { custom_features, .. } => {
                custom_features.contains(&feature.to_string())
            }
            ServiceTier::Professional { .. } => {
                matches!(feature, "analytics" | "api-access")
            }
            ServiceTier::Free { .. } => false,
        }
    }
}
```

## Caching Tier Information

Cache tier lookups to avoid database hits:

```rust
use moka::future::Cache;

pub struct CachedTierProvider {
    inner: Arc<dyn TierProvider>,
    cache: Cache<String, ServiceTier>,
}

impl CachedTierProvider {
    pub fn new(provider: Arc<dyn TierProvider>) -> Self {
        Self {
            inner: provider,
            cache: Cache::builder()
                .time_to_live(Duration::from_secs(300))
                .max_capacity(10_000)
                .build(),
        }
    }
}

impl TierProvider for CachedTierProvider {
    async fn get_tier(&self, tenant_id: &str) -> ServiceTier {
        if let Some(tier) = self.cache.get(tenant_id).await {
            return tier;
        }
        
        let tier = self.inner.get_tier(tenant_id).await;
        self.cache.insert(tenant_id.to_string(), tier.clone()).await;
        tier
    }
    
    async fn check_limits(&self, tenant_id: &str, operation: &Operation) -> Result<(), String> {
        self.inner.check_limits(tenant_id, operation).await
    }
}
```
