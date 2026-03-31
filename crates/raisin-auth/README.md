# raisin-auth

Pluggable authentication system for RaisinDB, inspired by [Passport.js](http://www.passportjs.org/).

## Overview

This crate provides a flexible, strategy-based authentication framework with:

- **Pluggable Strategies**: Local, Magic Link, One-Time Token, OIDC (Google, Okta, Azure AD, etc.)
- **Lean JWT + Hot Cache**: Minimal token payloads with LRU-cached workspace permissions
- **Session Management**: Server-side sessions with TTL-based expiration
- **Identity Linking**: Support for multiple auth providers per user
- **Workspace Access Control**: Fine-grained permission caching

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
│              │   AuthStrategyRegistry │                     │
│              └───────────┬───────────┘                      │
│                          ▼                                  │
│              ┌───────────────────────┐                      │
│              │      AuthService      │ (in raisin-core)     │
│              └───────────────────────┘                      │
└─────────────────────────────────────────────────────────────┘
```

## Strategies

| Strategy | Description | Credentials |
|----------|-------------|-------------|
| `LocalStrategy` | Username/password with bcrypt | `UsernamePassword` |
| `MagicLinkStrategy` | Passwordless email tokens | `MagicLinkToken` |
| `OneTimeTokenStrategy` | API keys, invites, resets | `OneTimeToken`, `ApiKey` |
| `OidcStrategy` | OAuth2/OIDC providers | `OAuth2Code`, `OAuth2RefreshToken` |

## Usage

```rust
use raisin_auth::{AuthStrategyRegistry, strategies::LocalStrategy};
use std::sync::Arc;

// Create registry and register strategies
let registry = AuthStrategyRegistry::new();
registry.register(Arc::new(LocalStrategy::new())).await;

// Set default strategy for login form
registry.set_default(StrategyId::new("local")).await;

// Find strategy by credential type
let creds = AuthCredentials::UsernamePassword {
    username: "user@example.com".into(),
    password: "secret".into(),
};
let strategy = registry.find_supporting(&creds).await;
```

### Password Hashing (Local Strategy)

```rust
use raisin_auth::strategies::LocalStrategy;

// Hash password for storage
let hash = LocalStrategy::hash_password("SecurePassword123")?;

// Verify during authentication
let valid = LocalStrategy::verify_password("SecurePassword123", &hash);
```

### Token Generation (Magic Link / One-Time Token)

```rust
use raisin_auth::strategies::{MagicLinkStrategy, OneTimeTokenStrategy};

// Magic link token (64 char hex)
let (token, hash, prefix) = MagicLinkStrategy::generate_token()?;

// API key with prefix
let (api_key, hash) = OneTimeTokenStrategy::generate_token("rdb_api")?;
// Result: "rdb_api_abc123..."
```

### Permission Caching

```rust
use raisin_auth::cache::{PermissionCache, CacheKey, CachedPermissions};
use std::time::Duration;

// Create cache with 1000 entries, 5 minute TTL
let cache = PermissionCache::new(1000, Duration::from_secs(300));

let key = CacheKey::new("session-123", "workspace-456");

// Get or resolve permissions
let perms = cache.get_or_resolve(key, || async {
    // Load from database if not cached
    Ok(CachedPermissions::new("user-789", vec!["admin".into()], vec![], true, 1))
}).await?;

// Invalidate on logout or permission change
cache.invalidate_session("session-123").await;
cache.invalidate_workspace("workspace-456").await;
```

## Features

```toml
[dependencies]
raisin-auth = { version = "0.1", features = ["oidc"] }
```

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `default` | Core strategies (local, magic_link, one_time_token) | - |
| `oidc` | OpenID Connect support | `reqwest` |

## Components

### Core (`src/`)

- `strategy.rs` - `AuthStrategy` trait and credential types
- `registry.rs` - `AuthStrategyRegistry` for strategy management
- `cache.rs` - LRU permission cache with TTL

### Strategies (`src/strategies/`)

- `local.rs` - Username/password with bcrypt
- `magic_link.rs` - Passwordless email authentication
- `one_time_token.rs` - API keys, invites, password reset
- `oidc.rs` - OAuth2/OIDC with PKCE support

### Jobs (`src/jobs/`)

Helpers for scheduling auth-related background tasks:

- `magic_link.rs` - Send magic link emails
- `session_cleanup.rs` - Expire stale sessions
- `token_cleanup.rs` - Remove expired tokens
- `access_notification.rs` - Notify workspace access changes

## Security Features

- **Bcrypt** password hashing (cost factor 12)
- **SHA-256** token hashing (never store plaintext)
- **PKCE** for OAuth2 authorization code flow
- **Constant-time** comparison for token verification
- **Account lockout** support (via AuthService)
- **LRU eviction** prevents unbounded cache growth

## Integration Notes

This crate provides authentication primitives. Full integration requires:

1. **Identity Store** - Persist user identities
2. **Session Store** - Manage server-side sessions
3. **Token Store** - Store one-time tokens
4. **AuthService** - Orchestrate strategies with storage (in `raisin-core`)

Jobs use the unified job queue pattern:
```rust
JobRegistry.register_job("auth_magic_link", ...)
JobDataStore.put(&job_id, &context)
```

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
