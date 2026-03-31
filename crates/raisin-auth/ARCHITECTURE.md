# Architecture

## Design Philosophy

raisin-auth follows the [Passport.js](http://www.passportjs.org/) pattern: authentication mechanisms are encapsulated as **strategies** that can be independently developed, tested, and composed.

## Core Abstractions

### AuthStrategy Trait

The foundation of the system. Each authentication method implements this trait:

```rust
#[async_trait]
pub trait AuthStrategy: Send + Sync {
    fn id(&self) -> &StrategyId;
    fn name(&self) -> &str;

    async fn init(&mut self, config: &AuthProviderConfig, secret: Option<&str>) -> Result<()>;
    async fn authenticate(&self, tenant_id: &str, creds: AuthCredentials) -> Result<AuthenticationResult>;

    // Redirect-based flows (OAuth2/OIDC/SAML)
    async fn get_authorization_url(&self, ...) -> Result<Option<String>>;
    async fn handle_callback(&self, ...) -> Result<AuthenticationResult>;

    fn supports(&self, credentials: &AuthCredentials) -> bool;
}
```

### AuthStrategyRegistry

Central registry for all authentication strategies:

```
┌────────────────────────────────────────────────┐
│            AuthStrategyRegistry                 │
│                                                 │
│  strategies: HashMap<StrategyId, Arc<dyn AuthStrategy>>
│  default_strategy: Option<StrategyId>          │
│                                                 │
│  Methods:                                       │
│  - register(strategy)                          │
│  - get(id) -> Option<Arc<dyn AuthStrategy>>    │
│  - find_supporting(creds) -> Option<...>       │
│  - initialize_all(configs, decrypt_fn)         │
└────────────────────────────────────────────────┘
```

### AuthCredentials

Sum type representing all possible authentication inputs:

```rust
pub enum AuthCredentials {
    UsernamePassword { username, password },
    MagicLinkToken { token },
    OneTimeToken { token },
    OAuth2Code { code, state, redirect_uri },
    OAuth2RefreshToken { refresh_token },
    ApiKey { key },
}
```

### AuthenticationResult

Normalized output from any successful authentication:

```rust
pub struct AuthenticationResult {
    pub identity_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub provider_claims: HashMap<String, Value>,
    pub external_id: Option<String>,
    pub strategy_id: StrategyId,
    pub provider_groups: Vec<String>,
    pub email_verified: bool,
    pub suggested_roles: Vec<String>,
}
```

## Authentication Flow

### Direct Authentication (Local, Token-based)

```
User                Strategy              AuthService
 │                      │                      │
 │──credentials────────>│                      │
 │                      │                      │
 │                      │──authenticate()─────>│
 │                      │                      │
 │                      │                      │ lookup identity
 │                      │                      │ verify password/token
 │                      │                      │ check lockout
 │                      │                      │
 │                      │<──AuthResult─────────│
 │<──tokens─────────────│                      │
```

### Redirect Authentication (OIDC/SAML)

```
User         App        Strategy           Provider
 │            │              │                  │
 │──login────>│              │                  │
 │            │──get_auth_url()───>│            │
 │            │<──url + state──────│            │
 │<──redirect─│                    │            │
 │                                             │
 │──────────────authenticate──────────────────>│
 │<─────────────redirect + code────────────────│
 │                                             │
 │──callback─>│              │                  │
 │            │──handle_callback()>│            │
 │            │              │──exchange code──>│
 │            │              │<──tokens─────────│
 │            │              │──userinfo───────>│
 │            │              │<──claims─────────│
 │            │<──AuthResult──────│            │
 │<──tokens───│              │                  │
```

## Permission Cache Architecture

```
┌─────────────────────────────────────────────────────┐
│                  PermissionCache                     │
│                                                      │
│  ┌─────────────────────────────────────────────┐    │
│  │           LRU Cache (RwLock)                 │    │
│  │                                              │    │
│  │  Key: (session_id, workspace_id)            │    │
│  │  Value: CachedPermissions                    │    │
│  │    - user_node_id                           │    │
│  │    - roles: Vec<String>                     │    │
│  │    - groups: Vec<String>                    │    │
│  │    - is_workspace_admin: bool               │    │
│  │    - resolved_at: Instant (for TTL)         │    │
│  │    - permissions_version: u64               │    │
│  └─────────────────────────────────────────────┘    │
│                                                      │
│  Invalidation:                                       │
│  - invalidate_session(id)  → O(n) scan             │
│  - invalidate_workspace(id) → O(n) scan            │
│  - TTL expiration          → on access             │
└─────────────────────────────────────────────────────┘
```

## Token Security Model

### Storage: Never Plaintext

```
User Token                    Database
─────────                     ────────
"rdb_api_abc123..."   ──SHA256──>   "7f8a9b..."
      │                              │
      └──── given to user            └──── stored in DB
```

### Verification

```rust
// Constant-time comparison prevents timing attacks
fn verify(user_token: &str, stored_hash: &str) -> bool {
    let computed = sha256(user_token);
    constant_time_eq(computed, stored_hash)
}
```

## Strategy Initialization

Strategies are initialized once at startup with decrypted secrets:

```
┌──────────────────────────────────────────────────────────────┐
│                        Startup                                │
│                                                               │
│  1. Load AuthProviderConfig from database/config              │
│  2. Decrypt client_secret_encrypted using app secret          │
│  3. For each strategy:                                        │
│     - Call strategy.init(config, decrypted_secret)           │
│     - Strategy stores config in OnceLock (interior mut)       │
│  4. Register with AuthStrategyRegistry                        │
└──────────────────────────────────────────────────────────────┘
```

## OIDC Discovery Flow

```
OidcStrategy.init()
      │
      ├──> issuer_url provided?
      │         │
      │         ├── Yes ──> Fetch /.well-known/openid-configuration
      │         │                │
      │         │                ├── Success ──> Use discovered endpoints
      │         │                │
      │         │                └── Failure ──> Fall back to manual config
      │         │
      │         └── No ──> Require manual endpoint URLs
      │
      └──> Store OidcConfig in OnceLock
```

## Job Integration Pattern

Auth jobs follow the unified job queue pattern:

```rust
// Create job data
let (job_type, context) = create_magic_link_job(
    tenant_id, identity_id, email, token
);

// Register job
let job_id = job_registry.register_job(
    job_type,
    Some(tenant_id.into()),
    None,  // scheduled_at
    None,  // priority
    Some(3), // max_retries
).await?;

// Store context
job_data_store.put(&job_id, &context)?;
```

## Multi-Tenancy

Every authentication operation is tenant-scoped:

- Strategies receive `tenant_id` in `authenticate()` and callback handlers
- Providers can be configured per-tenant via `TenantAuthConfig`
- Sessions and tokens are tenant-isolated
- Permission cache keys include workspace context
