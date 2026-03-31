# RaisinDB Pluggable Authentication Concept

## Executive Summary

This document outlines a **pluggable authentication architecture** for RaisinDB, modeled after the passport.js strategy pattern. The design separates **authentication** (global, identity verification) from **authorization** (per-workspace, permission enforcement) while integrating seamlessly with the existing ACL system in `raisin:access_control`.

---

## Architecture Overview

### Key Design Principles

1. **Pluggable Strategy Pattern**: Authentication providers are strategies implementing a common trait (like passport.js)
2. **Separation of Concerns**: Authentication is global; authorization is per-workspace
3. **Event-Driven**: Uses existing EventBus pattern for auth lifecycle events
4. **Job Queue Integration**: Magic links, token cleanup via JobRegistry
5. **System Workspace**: Global `raisin:system` workspace for cross-repository user storage
6. **JWT-First**: Optimized JWT structure with pre-resolved permissions for fast authorization

### Two-Tier Model

```
┌─────────────────────────────────────────────────────────────────┐
│                     AUTHENTICATION LAYER                        │
│  (Global - per tenant, cross-workspace)                        │
│                                                                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐             │
│  │   Local     │  │   OIDC      │  │  Magic Link │  ...more    │
│  │  Strategy   │  │  Strategy   │  │  Strategy   │             │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘             │
│         │                │                │                     │
│         └────────────────┼────────────────┘                     │
│                          ▼                                      │
│              ┌───────────────────────┐                          │
│              │   AuthStrategyRegistry │                         │
│              └───────────┬───────────┘                          │
│                          ▼                                      │
│              ┌───────────────────────┐                          │
│              │      AuthService      │                          │
│              │  - authenticate()     │                          │
│              │  - generate_jwt()     │                          │
│              │  - refresh_token()    │                          │
│              └───────────┬───────────┘                          │
│                          ▼                                      │
│              ┌───────────────────────┐                          │
│              │   Identity (raisin:   │                          │
│              │   system workspace)   │                          │
│              └───────────────────────┘                          │
└─────────────────────────────────────────────────────────────────┘
                           │
                           │ JWT with resolved permissions
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                     AUTHORIZATION LAYER                         │
│  (Per-workspace - raisin:access_control)                       │
│                                                                 │
│  ┌───────────────────────┐     ┌───────────────────────┐       │
│  │   Workspace A         │     │   Workspace B         │       │
│  │   raisin:access_ctrl  │     │   raisin:access_ctrl  │       │
│  │   ├── raisin:User     │     │   ├── raisin:User     │       │
│  │   ├── raisin:Role     │     │   ├── raisin:Role     │       │
│  │   └── raisin:Group    │     │   └── raisin:Group    │       │
│  └───────────────────────┘     └───────────────────────┘       │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐ │
│  │                   PermissionChecker                        │ │
│  │  - Uses JWT claims for fast path                          │ │
│  │  - Falls back to PermissionService for detailed checks    │ │
│  └───────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

---

## Core Trait: AuthStrategy

```rust
/// The core authentication strategy trait (passport.js pattern)
#[async_trait]
pub trait AuthStrategy: Send + Sync {
    /// Get the strategy identifier (e.g., "local", "oidc:okta")
    fn id(&self) -> &StrategyId;

    /// Get human-readable name for UI
    fn name(&self) -> &str;

    /// Initialize strategy with config and decrypted secrets
    /// Called once at startup - secrets resolved here, not per-request
    async fn init(
        &mut self,
        config: &AuthProviderConfig,
        decrypted_secret: Option<&str>,
    ) -> Result<()>;

    /// Authenticate with the given credentials
    async fn authenticate(
        &self,
        tenant_id: &str,
        credentials: AuthCredentials,
    ) -> Result<AuthenticationResult>;

    /// Get authorization URL for redirect-based flows (OAuth2/OIDC/SAML)
    async fn get_authorization_url(
        &self,
        tenant_id: &str,
        state: &str,
        redirect_uri: &str,
    ) -> Result<Option<String>>;

    /// Handle callback for redirect-based flows
    async fn handle_callback(
        &self,
        tenant_id: &str,
        params: HashMap<String, String>,
    ) -> Result<AuthenticationResult>;

    /// Handle logout (e.g., OIDC back-channel logout, token revocation)
    async fn handle_logout(&self, identity_id: &str) -> Result<()> {
        Ok(()) // Default: no-op for strategies without back-channel logout
    }
}
```

### Secret Management

Secrets are encrypted at rest (AES-256-GCM, like AI provider keys) and decrypted **once at init**, not per-request:

```rust
pub struct AuthProviderConfig {
    pub provider_id: String,
    pub strategy_id: StrategyId,
    pub display_name: String,
    pub icon: String,
    pub enabled: bool,

    // Encrypted at rest (AES-256-GCM)
    pub client_secret_encrypted: Option<Vec<u8>>,

    // Plaintext config
    pub issuer_url: Option<String>,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub attribute_mapping: AttributeMapping,
}

impl AuthStrategyRegistry {
    /// Initialize all strategies at startup (resolve secrets once)
    pub async fn initialize_all(
        &self,
        encryption_key: &[u8],
    ) -> Result<()> {
        for (id, strategy) in self.strategies.write().await.iter_mut() {
            let config = self.load_config(id).await?;

            // Decrypt secret once at init
            let decrypted = config.client_secret_encrypted.as_ref()
                .map(|enc| decrypt_aes256_gcm(enc, encryption_key))
                .transpose()?;

            strategy.init(&config, decrypted.as_deref()).await?;
        }
        Ok(())
    }
}
```

### Supported Strategies

| Strategy | Credentials | Use Case |
|----------|-------------|----------|
| `local` | Username/Password | Traditional login |
| `magic_link` | Email token | Passwordless login |
| `one_time_token` | Generated token | API access, invites |
| `oidc:google` | OAuth2 code | Google Workspace |
| `oidc:okta` | OAuth2 code | Okta enterprise SSO |
| `oidc:keycloak` | OAuth2 code | Self-hosted identity |
| `oidc:azure` | OAuth2 code | Microsoft 365 / Azure AD |
| `saml` | SAML assertion | Enterprise SAML providers |

---

## Data Models

### Identity (Global User)

Stored in `raisin:system` workspace - represents a unique person across all workspaces:

```rust
pub struct Identity {
    pub identity_id: String,              // UUID
    pub email: String,                    // Primary identifier (unique)
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub created_at: Timestamp,
    pub last_login_at: Option<Timestamp>,
    pub is_active: bool,

    /// Linked authentication providers (user can have multiple)
    pub linked_providers: Vec<LinkedProvider>,

    /// Local credentials (if using local auth)
    pub local_credentials: Option<LocalCredentials>,

    /// Custom metadata
    pub metadata: HashMap<String, Value>,
}

pub struct LinkedProvider {
    pub strategy_id: String,              // e.g., "oidc:google"
    pub external_id: String,              // Provider's user ID
    pub claims: HashMap<String, Value>,   // Provider-specific data
    pub linked_at: Timestamp,
}

pub struct LocalCredentials {
    pub password_hash: String,            // bcrypt
    pub must_change_password: bool,
    pub failed_attempts: u32,
    pub locked_until: Option<Timestamp>,
}
```

### Workspace Access

Links Identity to workspace-specific `raisin:User` nodes:

```rust
pub struct WorkspaceAccess {
    pub identity_id: String,
    pub repo_id: String,
    pub user_node_id: String,             // raisin:User node in raisin:access_control
    pub status: WorkspaceAccessStatus,    // Active, Pending, Denied, Revoked, Invited
    pub granted_at: Option<Timestamp>,
    pub granted_by: Option<String>,       // Admin who granted access
}
```

### JWT Claims (Lean - Avoid Fat JWT Problem)

**Problem with embedded permissions**: If a user belongs to 20 workspaces, JWT could exceed 4KB+ (HTTP header limits). Stale permissions until token refresh.

**Solution**: Lean JWT + Hot Permission Cache

```rust
// Lean JWT Claims (< 1KB)
pub struct AuthClaims {
    // Standard JWT claims
    pub sub: String,                      // identity_id
    pub iat: i64,
    pub exp: i64,
    pub jti: String,                      // For revocation

    // Custom claims
    pub email: String,
    pub tenant_id: String,
    pub auth_strategy: String,
    pub sid: String,                      // Session ID
    pub auth_time: i64,                   // For sudo mode (re-auth)

    pub global_flags: GlobalFlags,
    pub token_type: TokenType,            // Access, Refresh, Admin, Impersonation
}

pub struct GlobalFlags {
    pub is_tenant_admin: bool,
    pub email_verified: bool,
}
```

**Permission Resolution Flow:**
1. JWT contains only `identity_id`, `session_id`, `global_flags`
2. Active workspace passed via `X-Raisin-Workspace` header
3. Permissions cached in hot LRU cache keyed by `(session_id, workspace_id)`
4. Cache miss → resolve via `PermissionService` → cache result (5 min TTL)
5. Cache invalidation on role/permission changes via EventBus

```rust
// Permission cache (in-memory LRU)
pub struct PermissionCache {
    cache: Arc<RwLock<LruCache<(SessionId, WorkspaceId), CachedPermissions>>>,
    ttl: Duration,  // e.g., 5 minutes
}

pub struct CachedPermissions {
    pub user_node_id: String,             // raisin:User in this workspace
    pub roles: Vec<String>,               // Effective roles
    pub groups: Vec<String>,              // Group memberships
    pub is_workspace_admin: bool,
    pub resolved_at: Instant,
    pub permissions_version: u64,
}
```

### Sudo Mode (Re-authentication for Sensitive Operations)

**Purpose**: Certain sensitive operations require fresh authentication (password entered within last N minutes)

```rust
// JWT includes auth_time
pub auth_time: i64,  // Unix timestamp of actual authentication

// Middleware for sensitive operations
pub async fn require_fresh_auth(
    claims: &AuthClaims,
    max_age_seconds: i64,  // e.g., 300 = 5 minutes
) -> Result<(), AuthError> {
    let now = chrono::Utc::now().timestamp();
    if now - claims.auth_time > max_age_seconds {
        return Err(AuthError::ReauthenticationRequired);
    }
    Ok(())
}
```

**Operations requiring fresh auth:**
- Delete workspace
- Change password
- Link/unlink authentication providers
- Revoke all sessions
- Change tenant settings

### Identity Linking (Multi-Provider Support)

**Scenario**: User signs up with email/password, later wants to link Google for convenience

```rust
impl AuthService {
    /// Find by provider, or by email and link, or create new
    pub async fn find_and_link_or_create(
        &self,
        tenant_id: &str,
        auth_result: &AuthenticationResult,
    ) -> Result<Identity> {
        // 1. Check if provider already linked → return existing
        if let Some(id) = self.identity_store
            .find_by_provider(&auth_result.strategy_id, &auth_result.external_id)
            .await? {
            return self.identity_store.get(id).await;
        }

        // 2. Check if email exists → link provider to existing
        if let Some(existing) = self.identity_store
            .find_by_email(tenant_id, &auth_result.email)
            .await? {
            self.link_provider(&existing.identity_id, auth_result).await?;
            return Ok(existing);
        }

        // 3. Create new identity with this provider
        self.create_identity(tenant_id, auth_result).await
    }
}
```

---

## System Workspace (`raisin:system`)

A special global workspace per tenant for authentication data:

```yaml
name: raisin:system
description: System workspace for global authentication and configuration
scope: tenant  # Per-tenant but cross-repository
allowed_node_types:
  - raisin:Identity
  - raisin:Session
  - raisin:OneTimeToken
  - raisin:WorkspaceAccess
  - raisin:AuthConfig
  - raisin:ProviderConfig
```

### Storage Layout

```
sys\0{tenant_id}\0system\0identities\0{identity_id}        -> Identity
sys\0{tenant_id}\0system\0sessions\0{session_id}           -> Session
sys\0{tenant_id}\0system\0tokens\0{token_hash}             -> OneTimeToken
sys\0{tenant_id}\0system\0access\0{identity_id}\0{repo_id} -> WorkspaceAccess

# Indexes
sys\0{tenant_id}\0idx\0email\0{email}                      -> identity_id
sys\0{tenant_id}\0idx\0provider\0{strategy}\0{external_id} -> identity_id
```

---

## Authentication Flows

### 1. Local Authentication (Username/Password)

```
Client                    AuthService              IdentityStore
  │                           │                         │
  │ POST /auth/login          │                         │
  │ {email, password}         │                         │
  │──────────────────────────>│                         │
  │                           │ get_by_email()          │
  │                           │────────────────────────>│
  │                           │<────────────────────────│
  │                           │                         │
  │                           │ verify_password()       │
  │                           │────┐                    │
  │                           │<───┘                    │
  │                           │                         │
  │                           │ resolve_permissions()   │
  │                           │ (all workspaces)        │
  │                           │────────────────────────>│
  │                           │                         │
  │                           │ create_session()        │
  │                           │────────────────────────>│
  │                           │                         │
  │                           │ generate_jwt()          │
  │                           │────┐                    │
  │                           │<───┘                    │
  │<──────────────────────────│                         │
  │ {access_token,            │                         │
  │  refresh_token}           │                         │
```

### 2. OAuth2/OIDC Flow

```
Client          AuthService         OIDCStrategy         Provider
  │                  │                   │                  │
  │ GET /auth/oidc/  │                   │                  │
  │     {provider}   │                   │                  │
  │─────────────────>│                   │                  │
  │                  │ get_auth_url()    │                  │
  │                  │──────────────────>│                  │
  │<─────────────────│<──────────────────│                  │
  │ 302 Redirect     │                   │                  │
  │                  │                   │                  │
  │  ...User logs in with provider...   │                  │
  │                  │                   │                  │
  │ GET /auth/callback?code=xxx         │                  │
  │─────────────────>│                   │                  │
  │                  │ handle_callback() │                  │
  │                  │──────────────────>│                  │
  │                  │                   │ exchange_code()  │
  │                  │                   │─────────────────>│
  │                  │                   │<─────────────────│
  │                  │                   │ {id_token}       │
  │                  │<──────────────────│                  │
  │                  │                   │                  │
  │                  │ find_or_create_identity()           │
  │                  │ link_provider()   │                  │
  │                  │ resolve_permissions()               │
  │                  │ generate_jwt()    │                  │
  │<─────────────────│                   │                  │
  │ {access_token,   │                   │                  │
  │  refresh_token}  │                   │                  │
```

### 3. Magic Link Flow

```
Client          AuthService              JobQueue           Email
  │                  │                       │                │
  │ POST /auth/magic-link                    │                │
  │ {email}          │                       │                │
  │─────────────────>│                       │                │
  │                  │ create_token()        │                │
  │                  │────┐                  │                │
  │                  │<───┘                  │                │
  │                  │ enqueue_email_job()   │                │
  │                  │──────────────────────>│                │
  │<─────────────────│                       │───────────────>│
  │ 202 Accepted     │                       │  Send email    │
  │                  │                       │                │
  │  ...User clicks link...                  │                │
  │                  │                       │                │
  │ GET /auth/magic-link/verify?token=xxx   │                │
  │─────────────────>│                       │                │
  │                  │ validate_token()      │                │
  │                  │ find_or_create_identity()             │
  │                  │ mark_email_verified() │                │
  │                  │ generate_jwt()        │                │
  │<─────────────────│                       │                │
  │ {access_token,   │                       │                │
  │  refresh_token}  │                       │                │
```

### 4. Workspace Access Request Flow

```
Client             AccessService            EventBus           Admin
  │                     │                       │                │
  │ POST /repos/{repo}/access/request          │                │
  │ Authorization: Bearer <jwt>                │                │
  │────────────────────>│                       │                │
  │                     │ create WorkspaceAccess│                │
  │                     │ (status=Pending)      │                │
  │                     │                       │                │
  │                     │ publish(AccessRequested)              │
  │                     │──────────────────────>│───────────────>│
  │<────────────────────│                       │  Notification  │
  │ 202 Accepted        │                       │                │
  │                     │                       │                │
  │  ...Admin approves in admin-console...     │                │
  │                     │                       │                │
  │                     │ approve_access()      │                │
  │                     │────┐                  │                │
  │                     │<───┘                  │                │
  │                     │ - update status=Active│                │
  │                     │ - create raisin:User  │                │
  │                     │ - link to Identity    │                │
  │                     │                       │                │
  │                     │ publish(AccessGranted)│                │
  │                     │──────────────────────>│───────────────>│
  │                     │                       │  Notify user   │
```

---

## Module Structure

```
crates/
├── raisin-auth/                        # NEW CRATE
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── strategy.rs                 # AuthStrategy trait
│       ├── registry.rs                 # AuthStrategyRegistry
│       ├── service.rs                  # AuthService
│       ├── claims.rs                   # JWT claims
│       ├── tokens.rs                   # Token generation/validation
│       ├── session.rs                  # Session management
│       ├── strategies/
│       │   ├── mod.rs
│       │   ├── local.rs                # Username/password
│       │   ├── magic_link.rs           # Magic link emails
│       │   ├── one_time_token.rs       # One-time tokens
│       │   ├── oidc.rs                 # Generic OIDC
│       │   └── saml.rs                 # SAML 2.0 (future)
│       ├── stores/
│       │   ├── mod.rs
│       │   ├── identity.rs             # Identity storage
│       │   ├── session.rs              # Session storage
│       │   └── token.rs                # Token storage
│       └── events.rs                   # Auth events
│
├── raisin-models/src/auth/
│   ├── identity.rs                     # NEW: Identity model
│   ├── session.rs                      # NEW: Session model
│   ├── claims.rs                       # NEW: JWT claims
│   └── context.rs                      # UPDATED: AuthContext
│
├── raisin-core/global_workspaces/
│   └── system.yaml                     # NEW: System workspace
│
└── raisin-transport-http/src/
    ├── handlers/auth.rs                # UPDATED: Auth endpoints
    └── middleware/auth.rs              # UPDATED: Unified middleware
```

---

## Integration Points

### 1. Current x-raisin-impersonation

**Kept as-is** for admin users. The new system adds:
- Regular user impersonation via `TokenType::Impersonation`
- Impersonation audit trail in session data

### 2. Existing AuthService

**Refactored** to use new strategy pattern:
- `AdminClaims` remains for database admin users (operators)
- `AuthClaims` added for application users
- Both validated through unified middleware

### 3. Permission Resolution

**Optimized** by pre-resolving into JWT:
- On login: resolve permissions for all accessible workspaces
- Store in `workspace_permissions` claim
- On refresh: re-resolve if permissions might have changed
- Fallback: `PermissionService` for detailed condition evaluation

### 4. Event System

**New events** for authentication lifecycle:
- `AuthEvent::Login { identity_id, strategy, session_id }`
- `AuthEvent::Logout { identity_id, session_id }`
- `AuthEvent::TokenRefresh { identity_id, session_id }`
- `AuthEvent::PasswordChanged { identity_id }`
- `AccessEvent::Requested { identity_id, repo_id }`
- `AccessEvent::Granted { identity_id, repo_id, granted_by }`
- `AccessEvent::Revoked { identity_id, repo_id, revoked_by }`

---

## Configuration

### Provider Configuration (stored in raisin:system)

```yaml
# raisin:ProviderConfig node
node_type: raisin:ProviderConfig
name: google-workspace
properties:
  strategy_id: "oidc:google"
  enabled: true
  display_name: "Sign in with Google"
  icon: "google"
  priority: 1
  config:
    issuer_url: "https://accounts.google.com"
    client_id: "xxx.apps.googleusercontent.com"
    client_secret: "$SECRET:google_client_secret"  # Reference to secret store
    scopes: ["openid", "email", "profile"]
    attribute_mapping:
      email: "email"
      name: "name"
      picture: "picture"
    groups_claim: "groups"  # If using Google Groups
```

### Auth Settings (stored in raisin:system)

```yaml
# raisin:AuthConfig node
node_type: raisin:AuthConfig
name: config
properties:
  session_duration_hours: 24
  refresh_token_duration_days: 30
  max_sessions_per_user: 10

  password_policy:
    min_length: 12
    require_uppercase: true
    require_lowercase: true
    require_digit: true
    require_special: true

  magic_link:
    enabled: true
    expiration_minutes: 15

  rate_limiting:
    max_attempts_per_minute: 5
    lockout_duration_minutes: 15
```

---

## Implementation Phases

### Phase 1: Foundation
- Create `raisin-auth` crate structure
- Define `AuthStrategy` trait and `AuthStrategyRegistry`
- Implement `Identity`, `Session`, `WorkspaceAccess` models
- Implement storage (IdentityStore, SessionStore)
- Create `raisin:system` workspace definition
- Implement `LocalStrategy` (username/password)
- Update `AuthClaims` with new structure
- Basic JWT generation with workspace permissions

### Phase 2: Session Management
- Session creation and lifecycle
- Refresh token flow with rotation
- Token revocation (logout, security)
- Session cleanup job via JobRegistry
- Session listing API for users

### Phase 3: Magic Links & Tokens
- Implement `MagicLinkStrategy`
- Implement `OneTimeTokenStrategy`
- Email job integration
- Token expiration cleanup

### Phase 4: OAuth2/OIDC
- Generic `OidcStrategy` implementation
- Provider configurations:
  - Google Workspace
  - Okta
  - Keycloak
  - Azure AD
- Provider linking (multiple auth methods per user)
- Group/role mapping from providers

### Phase 5: Workspace Access
- Access request flow
- Admin approval workflow
- Invitation system
- Event notifications
- Admin console UI

### Phase 6: Advanced
- MFA support
- SAML 2.0
- Rate limiting
- Audit logging
- Admin console configuration UI

---

## Security Considerations

1. **Password Hashing**: bcrypt with cost factor 12+
2. **Token Security**:
   - Short-lived access tokens (1 hour)
   - Long-lived refresh tokens (30 days) with rotation
   - Token family tracking for refresh token reuse detection
3. **Session Security**:
   - Server-side session validation
   - IP/User-Agent tracking (optional)
   - Concurrent session limits
4. **Rate Limiting**: Per-IP and per-account limits
5. **Account Lockout**: After N failed attempts
6. **Audit Trail**: All auth events logged

---

## Open Questions

1. **Identity Scope**: Should identities be per-tenant or truly global (cross-tenant)?
2. **Email Uniqueness**: Email unique per-tenant or globally?
3. **Initial Implementation**: Start with local + one OIDC provider, or full suite?
4. **Admin Console Priority**: How important is UI configuration vs code/YAML config?

---

## Comparison with passport.js

| Passport.js | RaisinDB Auth |
|-------------|---------------|
| `passport.use(strategy)` | `registry.register(strategy)` |
| `passport.authenticate('local')` | `service.authenticate(credentials)` |
| `serializeUser` / `deserializeUser` | `Identity` in system workspace |
| `req.user` | `AuthContext` in request extensions |
| Session-based | JWT + server-side sessions |
| Express middleware | Axum/Tower middleware |

The architecture follows the same principle: **pluggable strategies** that produce a **unified user identity**, but adapted for:
- Rust's async/trait system
- RaisinDB's existing patterns (TenantResolver, EventBus, JobRegistry)
- JWT-first approach for distributed authorization
