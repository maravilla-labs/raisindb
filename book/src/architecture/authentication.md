# Authentication

RaisinDB provides a pluggable authentication system inspired by Passport.js, implemented in the `raisin-auth` crate. Rather than coupling authentication to a single provider, the system defines a strategy abstraction that lets each tenant mix and match providers -- local passwords, magic links, OIDC, API keys -- while sharing a unified session and token infrastructure underneath.

This chapter covers the full authentication pipeline: from how credentials enter the system, through identity resolution and token issuance, to session lifecycle management. For what happens *after* authentication -- workspace-level roles, permissions, and enforcement -- see the [Access Control](./access-control.md) chapter.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     AUTHENTICATION LAYER                    │
│                                                             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │   Local     │  │   OIDC      │  │  Magic Link │  ...    │
│  │  Strategy   │  │  Strategy   │  │  Strategy   │         │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘         │
│         └────────────────┼────────────────┘                 │
│                          ▼                                  │
│              ┌───────────────────────┐                      │
│              │  AuthStrategyRegistry │                      │
│              └───────────┬───────────┘                      │
│                          ▼                                  │
│              ┌───────────────────────┐                      │
│              │      AuthService      │                      │
│              │  (JWT + Sessions)     │                      │
│              └───────────────────────┘                      │
└─────────────────────────────────────────────────────────────┘
```

Every authentication attempt follows the same flow regardless of strategy:

1. The transport layer receives credentials and wraps them in an `AuthCredentials` variant.
2. The `AuthStrategyRegistry` routes the credentials to the matching strategy.
3. The strategy validates the credentials and returns a verified `Identity`.
4. `AuthService` creates or updates a `Session`, then issues JWT access and refresh tokens.

This separation means adding a new authentication method (say, SAML) requires only a new strategy implementation -- the session, token, and middleware infrastructure stays untouched.

## Authentication Strategies

Each strategy maps to one or more variants of the `AuthCredentials` enum:

| Strategy | Credential Variant | Use Case |
|----------|-------------------|----------|
| Local | `UsernamePassword` | Traditional email + password login |
| Magic Link | `MagicLinkToken` | Passwordless email authentication |
| OIDC | `OAuth2Code`, `OAuth2RefreshToken` | Google, Okta, Keycloak, Azure AD |
| One-Time Token | `OneTimeToken` | Email verification, password reset, invitations |
| API Key | `ApiKey` | Machine-to-machine access |

### Local Strategy

Username and password authentication with bcrypt password hashing:

```rust
use raisin_auth::{AuthStrategyRegistry, AuthCredentials};
use raisin_auth::strategies::LocalStrategy;

let registry = AuthStrategyRegistry::new();
registry.register(Arc::new(LocalStrategy::new())).await;

// Authenticate with username/password
let result = auth_service.authenticate(
    "tenant-1",
    AuthCredentials::UsernamePassword {
        username: "user@example.com".to_string(),
        password: "secure-password".to_string(),
    },
).await?;
```

Password complexity is enforced by a configurable policy (see [AuthConfig](#tenant-level-configuration) below). The policy controls minimum and maximum length, and whether uppercase, lowercase, digit, and special characters are required.

### Magic Link Strategy

Passwordless authentication via email-based one-time tokens. The user requests a magic link, receives it by email, and clicking the link completes authentication without ever setting a password:

```rust
let credentials = AuthCredentials::MagicLinkToken {
    token: "one-time-token-from-email".to_string(),
};
```

Magic links are configurable per tenant -- they can be enabled or disabled, and their expiration time is adjustable (see the `magic_link` section of `AuthConfig`).

### OIDC Strategy

OpenID Connect integration with support for:

- **Google** -- Google Workspace and consumer accounts
- **Okta** -- Enterprise identity provider
- **Keycloak** -- Open-source identity management
- **Azure AD** -- Microsoft identity platform

The OIDC strategy requires the `oidc` feature flag:

```toml
[dependencies]
raisin-auth = { path = "../raisin-auth", features = ["oidc"] }
```

The HTTP transport exposes two endpoints per provider: one to start the OAuth2 flow (`GET /auth/oidc/{provider}`) and one to handle the callback (`GET /auth/oidc/{provider}/callback`). On successful callback, the strategy resolves the provider's user ID to a local `Identity`, creating one if it does not exist, and links the provider via `LinkedProvider`.

### One-Time Token Strategy

Short-lived tokens for specific purposes like email verification, password reset, or workspace invitations. These tokens are single-use and expire quickly, making them suitable for sensitive operations.

### API Key Strategy

Long-lived credentials for machine-to-machine access. API keys authenticate as an identity but bypass the interactive login flow, making them suitable for CI/CD pipelines, scripts, and service integrations.

## Identity

The `Identity` is RaisinDB's representation of a person or service account within a tenant. It is the anchor point that ties together all authentication methods and sessions for a single user.

```rust
pub struct Identity {
    pub identity_id: String,
    pub tenant_id: String,
    pub email: String,
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub is_active: bool,
    pub linked_providers: Vec<LinkedProvider>,
    pub local_credentials: Option<LocalCredentials>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub created_at: StorageTimestamp,
    pub last_login_at: Option<StorageTimestamp>,
}
```

Key properties:

| Field | Purpose |
|-------|---------|
| `identity_id` | Globally unique identifier within the tenant |
| `email_verified` | Whether the email has been confirmed; used in `GlobalFlags` on tokens |
| `is_active` | Disabled identities cannot authenticate |
| `linked_providers` | External providers (OIDC, etc.) linked to this identity |
| `local_credentials` | Bcrypt-hashed password for local strategy |
| `metadata` | Arbitrary key-value data attached by the application |

### Identity Linking

A single identity can be linked to multiple authentication providers. For example, a user might authenticate with both Google OIDC and a local password. Each link is tracked by a `LinkedProvider`:

```rust
pub struct LinkedProvider {
    pub strategy_id: String,      // e.g., "oidc:google"
    pub external_id: String,      // Provider's user ID
    pub display_name: Option<String>,
    pub linked_at: StorageTimestamp,
}
```

The `strategy_id` uses a namespaced format (`oidc:google`, `oidc:okta`) so that multiple OIDC providers can coexist without collision. When an OIDC callback returns, the system first searches for an existing identity with a matching `(strategy_id, external_id)` pair before creating a new one.

## JWT Tokens

Authentication produces a pair of tokens with different lifetimes and purposes:

- **Access Token** (1 hour) -- Short-lived JWT containing user claims. Validated on every request by the auth middleware. Kept intentionally compact for fast validation.
- **Refresh Token** (30 days) -- Longer-lived token for obtaining new access tokens without re-authentication. Supports rotation for security.

### AuthClaims

The access token carries the following claims:

```rust
pub struct AuthClaims {
    pub sub: String,              // identity_id
    pub email: String,
    pub tenant_id: String,
    pub repository: Option<String>,
    pub home: Option<String>,     // raisin:User node path for path-based ACL
    pub sid: String,              // session_id
    pub auth_strategy: String,
    pub auth_time: i64,           // For sudo mode re-auth
    pub global_flags: GlobalFlags,
    pub token_type: TokenType,
    pub exp: i64,
    pub iat: i64,
    pub nbf: Option<i64>,
    pub jti: String,
    pub iss: Option<String>,
    pub aud: Option<String>,
}
```

A few claims deserve explanation:

| Claim | Purpose |
|-------|---------|
| `home` | Points to the `raisin:User` node for this identity in a workspace. Used by the [Access Control](./access-control.md) layer for path-based ACL resolution. |
| `auth_time` | Records when the user last actively authenticated (not refreshed). Enables sudo mode -- sensitive operations can require that `auth_time` be within the `sudo_threshold_seconds` window. |
| `auth_strategy` | Which strategy produced this token (`local`, `oidc:google`, etc.). Policies can require specific strategies for sensitive operations. |
| `global_flags` | Tenant-wide flags that travel with every request (see below). |
| `token_type` | Distinguishes access, refresh, and impersonation tokens. |

Note that **workspace permissions are NOT stored in the JWT**. They are resolved per-request via an LRU cache with a 5-minute TTL, keeping tokens small and permissions always fresh. See [Access Control](./access-control.md) for details.

### GlobalFlags

```rust
pub struct GlobalFlags {
    pub is_tenant_admin: bool,
    pub email_verified: bool,
    pub must_change_password: bool,
}
```

These flags are embedded in every access token so the middleware can enforce tenant-wide policies (like blocking unverified emails or forcing password changes) without additional storage lookups.

### TokenType

```rust
pub enum TokenType {
    Access,
    Refresh,
    Impersonation { original_user_id: String },
}
```

The `Impersonation` variant supports admin debugging workflows. When an admin impersonates a user (via the `X-Raisin-Impersonate` header), the resulting token records the original admin's identity so audit logs can distinguish impersonated actions from real ones.

## Session Management

Every successful authentication creates a server-side `Session`. Sessions are the ground truth for whether a user is logged in -- even if a JWT has not yet expired, the middleware checks that the referenced session has not been revoked.

```rust
pub struct Session {
    pub session_id: String,
    pub tenant_id: String,
    pub identity_id: String,
    pub strategy_id: String,
    pub created_at: StorageTimestamp,
    pub expires_at: StorageTimestamp,
    pub last_activity_at: StorageTimestamp,
    pub client_info: ClientInfo,
    pub revoked: bool,
    pub refresh_token_hash: Option<String>,
    pub token_family: String,
    pub token_generation: u32,
}
```

### Refresh Token Rotation

When `rotate_refresh_tokens` is enabled (the default), each use of a refresh token issues a new refresh token and increments `token_generation`. The old refresh token becomes invalid immediately.

The `token_family` field ties all refresh tokens in a rotation chain to the same root. If a previously-used refresh token is presented again -- indicating it was stolen and the legitimate user already rotated past it -- the system detects this as **token reuse**. When `revoke_on_reuse_detection` is enabled, the entire token family is revoked, forcing re-authentication on all devices that share the compromised chain.

### Session Limits

The `max_sessions_per_user` setting (default: 10) caps how many concurrent sessions a single identity can hold. When the limit is reached, the oldest session is revoked to make room for the new one.

## HTTP API Endpoints

### Identity Authentication

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/auth/providers` | List available authentication providers for the tenant |
| `POST` | `/auth/register` | Register a new user with local credentials |
| `POST` | `/auth/login` | Authenticate with credentials |
| `POST` | `/auth/magic-link` | Request a magic link email |
| `GET` | `/auth/magic-link/verify` | Verify a magic link token |
| `GET` | `/auth/oidc/{provider}` | Start an OIDC authentication flow |
| `GET` | `/auth/oidc/{provider}/callback` | Handle the OIDC provider callback |
| `POST` | `/auth/refresh` | Exchange a refresh token for a new access token |
| `POST` | `/auth/logout` | Logout and revoke the current session |
| `GET` | `/auth/sessions` | List all sessions for the current user |
| `DELETE` | `/auth/sessions/{session_id}` | Revoke a specific session |
| `GET` | `/auth/me` | Return the current user's identity and claims |

### Workspace Access

These endpoints manage who can access a workspace. For the permission model within a workspace (roles, groups, node-level permissions), see [Access Control](./access-control.md).

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/repos/{repo}/access/request` | Request access to a workspace |
| `GET` | `/repos/{repo}/access/requests` | List pending access requests |
| `POST` | `/repos/{repo}/access/approve/{request_id}` | Approve an access request |
| `POST` | `/repos/{repo}/access/deny/{request_id}` | Deny an access request |
| `POST` | `/repos/{repo}/access/invite` | Invite a user to a workspace |
| `POST` | `/repos/{repo}/access/revoke/{identity_id}` | Revoke a user's workspace access |
| `GET` | `/repos/{repo}/access/members` | List workspace members |

## Auth Middleware

The HTTP transport layer uses two middleware variants to protect routes:

1. **`require_auth_middleware`** -- Validates the JWT, checks that the referenced session is active, and injects `AuthClaims` into the request context. Returns `401 Unauthorized` if the token is missing, expired, or the session is revoked.

2. **`optional_auth_middleware`** -- Performs the same validation when a token is present, but allows the request to proceed anonymously if no token is provided. Used for public content endpoints where authentication enhances but does not gate access.

Both middleware variants support **dual JWT validation** -- they accept tokens signed by either the tenant's user signing key or the admin signing key. This allows admin tooling and user-facing applications to share the same API surface.

**Admin impersonation** is supported via the `X-Raisin-Impersonate` header. When an admin-signed token includes this header, the middleware resolves the target user's identity and issues an `Impersonation` token type, preserving the original admin's ID for audit purposes.

## Tenant-Level Configuration

Each tenant configures authentication independently through the `AuthConfig` node type. This is stored as a node in the tenant's system workspace, making it versionable and replicable like any other data.

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `session_duration_hours` | Number | 24 | How long a session remains valid |
| `refresh_token_duration_days` | Number | 30 | Refresh token lifetime |
| `max_sessions_per_user` | Number | 10 | Concurrent session limit per identity |
| `sudo_threshold_seconds` | Number | 300 | Max age of `auth_time` for sensitive operations |
| `rotate_refresh_tokens` | Boolean | true | Issue new refresh token on each use |
| `revoke_on_reuse_detection` | Boolean | true | Revoke token family on reuse detection |
| `audit_enabled` | Boolean | true | Log authentication events |
| `anonymous_enabled` | Boolean | false | Allow unauthenticated access |

### Password Policy

Nested under `password_policy`:

| Property | Description |
|----------|-------------|
| `min_length` | Minimum password length |
| `max_length` | Maximum password length |
| `require_uppercase` | Require at least one uppercase letter |
| `require_lowercase` | Require at least one lowercase letter |
| `require_digit` | Require at least one digit |
| `require_special` | Require at least one special character |

### Magic Link Settings

Nested under `magic_link`:

| Property | Description |
|----------|-------------|
| `enabled` | Whether magic link authentication is available |
| `expiration_minutes` | How long a magic link remains valid |

### Rate Limiting

Nested under `rate_limiting`:

| Property | Description |
|----------|-------------|
| `max_attempts_per_minute` | Throttle for authentication attempts |
| `lockout_duration_minutes` | How long an account is locked after too many failures |
| `lockout_threshold` | Number of failures before lockout triggers |

This per-tenant configuration allows different tenants on the same RaisinDB instance to use different authentication providers, password policies, and security settings without any code changes.
