# Custom Tenant Resolver Example

Examples of custom tenant resolution strategies.

## JWT-Based Resolver

Extract tenant from JWT token:

```rust
use raisin_context::{TenantResolver, TenantContext};
use jsonwebtoken::{decode, DecodingKey, Validation};

#[derive(serde::Deserialize)]
struct Claims {
    tenant_id: String,
    deployment: Option<String>,
}

pub struct JwtTenantResolver {
    decoding_key: DecodingKey,
}

impl JwtTenantResolver {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            decoding_key: DecodingKey::from_secret(secret),
        }
    }
}

impl TenantResolver for JwtTenantResolver {
    fn resolve(&self, token: &str) -> Option<TenantContext> {
        let validation = Validation::default();
        
        let token_data = decode::<Claims>(
            token,
            &self.decoding_key,
            &validation,
        ).ok()?;
        
        Some(TenantContext::new(
            token_data.claims.tenant_id,
            token_data.claims.deployment.unwrap_or("production".to_string()),
        ))
    }
}
```

Usage:

```rust
let resolver = JwtTenantResolver::new(b"your-secret");
let ctx = resolver.resolve("eyJ0eXAiOiJKV1QiLCJh...");
```

## Header-Based Resolver

Extract from HTTP header:

```rust
pub struct HeaderTenantResolver;

impl TenantResolver for HeaderTenantResolver {
    fn resolve(&self, header_value: &str) -> Option<TenantContext> {
        // Parse format: "tenant_id:deployment"
        let parts: Vec<&str> = header_value.split(':').collect();
        
        match parts.as_slice() {
            [tenant_id, deployment] => {
                Some(TenantContext::new(*tenant_id, *deployment))
            }
            [tenant_id] => {
                Some(TenantContext::new(*tenant_id, "production"))
            }
            _ => None,
        }
    }
}
```

Usage with Axum:

```rust
use axum::http::HeaderMap;

async fn handler(headers: HeaderMap) -> Result<(), StatusCode> {
    let tenant_header = headers
        .get("X-Tenant-ID")
        .and_then(|h| h.to_str().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    
    let resolver = HeaderTenantResolver;
    let ctx = resolver
        .resolve(tenant_header)
        .ok_or(StatusCode::BAD_REQUEST)?;
    
    // Use ctx...
    Ok(())
}
```

## Path-Based Resolver

Extract from URL path:

```rust
pub struct PathTenantResolver;

impl TenantResolver for PathTenantResolver {
    fn resolve(&self, path: &str) -> Option<TenantContext> {
        // Parse format: "/tenants/{tenant_id}/..."
        let parts: Vec<&str> = path.split('/').collect();
        
        if parts.len() >= 3 && parts[1] == "tenants" {
            Some(TenantContext::new(parts[2], "production"))
        } else {
            None
        }
    }
}
```

Usage with Axum:

```rust
use axum::extract::Path;

async fn handler(Path(tenant_id): Path<String>) -> Result<(), StatusCode> {
    let ctx = TenantContext::new(tenant_id, "production");
    // Use ctx...
    Ok(())
}
```

## Database-Based Resolver

Look up tenant in database:

```rust
use sqlx::PgPool;

pub struct DatabaseTenantResolver {
    pool: PgPool,
}

impl DatabaseTenantResolver {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    
    async fn resolve_async(&self, api_key: &str) -> Option<TenantContext> {
        let result = sqlx::query!(
            "SELECT tenant_id, deployment FROM api_keys WHERE key = $1",
            api_key
        )
        .fetch_one(&self.pool)
        .await
        .ok()?;
        
        Some(TenantContext::new(
            result.tenant_id,
            result.deployment.unwrap_or("production".to_string()),
        ))
    }
}

// Note: TenantResolver trait is sync, so you'd need to adapt this
// or use a different pattern for async resolution
```

## Composite Resolver

Try multiple strategies:

```rust
use raisin_context::{TenantResolver, TenantContext};

pub struct CompositeResolver {
    resolvers: Vec<Box<dyn TenantResolver>>,
}

impl CompositeResolver {
    pub fn new() -> Self {
        Self {
            resolvers: vec![
                Box::new(HeaderTenantResolver),
                Box::new(PathTenantResolver),
            ],
        }
    }
}

impl TenantResolver for CompositeResolver {
    fn resolve(&self, input: &str) -> Option<TenantContext> {
        for resolver in &self.resolvers {
            if let Some(ctx) = resolver.resolve(input) {
                return Some(ctx);
            }
        }
        None
    }
}
```
