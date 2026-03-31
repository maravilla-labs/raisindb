# Roadmap

## Current Status

raisin-auth provides core authentication primitives tightly integrated with RaisinDB. The strategies are functional but rely on `raisin-core` AuthService for full orchestration.

## Future Enhancements

### Near-Term

- [ ] **SAML 2.0 Strategy** - Enterprise SSO support
- [ ] **WebAuthn/Passkey Strategy** - Hardware key and biometric authentication
- [ ] **Rate Limiting Integration** - Per-tenant/IP rate limits on auth endpoints
- [ ] **Audit Logging** - Integration with raisin-audit for auth events

### Medium-Term

- [ ] **Distributed Cache Backend** - Redis/Memcached support for PermissionCache
- [ ] **Secondary Cache Indexes** - O(1) session/workspace invalidation
- [ ] **MFA Support** - TOTP, SMS, authenticator app second factors
- [ ] **Password Policy Enforcement** - Complexity rules, breach detection
- [ ] **Session Binding** - Device fingerprinting, IP pinning

---

## Standalone Package Considerations

The following analysis documents what would be needed to publish raisin-auth as a standalone crate on crates.io, independent of RaisinDB.

### Dependencies to Abstract

| Dependency | Current State | Effort |
|------------|---------------|--------|
| `raisin-error` | Clean, minimal | Easy - extract or use generic `Result` |
| `raisin-models` | Heavy coupling (14+ types) | Hard - requires model extraction |

### Required Abstractions

#### Easy (1-2 days)

- [ ] Extract error types or use `Box<dyn Error>`
- [ ] Add feature flags for optional strategies
- [ ] Create complete runnable examples
- [ ] Update license (BSL-1.1 → Apache 2.0/MIT for wider adoption)

#### Medium (3-5 days)

- [ ] **IdentityStore Trait** - Abstract identity persistence
  ```rust
  #[async_trait]
  pub trait IdentityStore: Send + Sync {
      async fn find_by_email(&self, email: &str) -> Result<Option<Identity>>;
      async fn find_by_id(&self, id: &str) -> Result<Option<Identity>>;
      async fn create(&self, identity: Identity) -> Result<Identity>;
      async fn update(&self, identity: Identity) -> Result<Identity>;
  }
  ```

- [ ] **SessionStore Trait** - Abstract session persistence
  ```rust
  #[async_trait]
  pub trait SessionStore: Send + Sync {
      async fn create(&self, session: Session) -> Result<Session>;
      async fn find(&self, id: &str) -> Result<Option<Session>>;
      async fn refresh(&self, id: &str, new_expiry: DateTime) -> Result<()>;
      async fn revoke(&self, id: &str) -> Result<()>;
  }
  ```

- [ ] **TokenStore Trait** - Abstract token persistence
  ```rust
  #[async_trait]
  pub trait TokenStore: Send + Sync {
      async fn store(&self, token: OneTimeToken) -> Result<()>;
      async fn find_by_hash(&self, hash: &str) -> Result<Option<OneTimeToken>>;
      async fn mark_used(&self, id: &str) -> Result<()>;
      async fn cleanup_expired(&self) -> Result<u64>;
  }
  ```

- [ ] **NotificationSender Trait** - Abstract email/notification delivery
  ```rust
  #[async_trait]
  pub trait NotificationSender: Send + Sync {
      async fn send_magic_link(&self, email: &str, token: &str, url: &str) -> Result<()>;
      async fn send_password_reset(&self, email: &str, token: &str, url: &str) -> Result<()>;
  }
  ```

- [ ] Extract RaisinDB-specific fields from models (e.g., `repository`, `home` from AuthClaims)

#### Hard (1-2 weeks)

- [ ] **JobQueue Trait** - Abstract background job scheduling
  ```rust
  pub trait JobQueue: Send + Sync {
      fn enqueue(&self, job_type: &str, payload: Value) -> Result<JobId>;
      fn schedule(&self, job_type: &str, payload: Value, at: DateTime) -> Result<JobId>;
  }
  ```

- [ ] **DistributedCache Trait** - Abstract permission caching
  ```rust
  #[async_trait]
  pub trait PermissionCache: Send + Sync {
      async fn get(&self, key: &CacheKey) -> Option<CachedPermissions>;
      async fn set(&self, key: CacheKey, perms: CachedPermissions);
      async fn invalidate_session(&self, session_id: &str);
      async fn invalidate_workspace(&self, workspace_id: &str);
  }
  ```

- [ ] Full model extraction - Clone necessary types from `raisin-models` into this crate

### Proposed Feature Flags

```toml
[features]
default = ["local", "magic_link", "one_time_token"]
local = []
magic_link = []
one_time_token = []
oidc = ["dep:reqwest"]
saml = ["dep:samael"]  # future
webauthn = ["dep:webauthn-rs"]  # future

# Storage backend features
memory-store = []  # In-memory implementations for testing
redis-cache = ["dep:redis"]  # future

# RaisinDB integration (keeps current behavior)
raisindb = ["dep:raisin-models", "dep:raisin-error"]
```

### Breaking Changes for 1.0

If pursuing standalone publication:

1. `AuthenticationResult` would use generic claims instead of RaisinDB-specific fields
2. Job helpers would take trait objects instead of producing RaisinDB-specific data structures
3. Re-exports from `raisin-models` would be removed or feature-gated
4. Cache invalidation would use trait methods instead of direct struct access

### Estimated Timeline

| Phase | Scope | Effort |
|-------|-------|--------|
| **Phase 1** | Extract errors, add traits, mock impls, examples | 1-2 weeks |
| **Phase 2** | Full model decoupling, job abstraction | 2-3 weeks |
| **Phase 3** | Distributed cache, additional strategies | 2-4 weeks |

### Alternative: Keep Coupled

If standalone publication is not a priority, the current architecture is sound for RaisinDB-only use. Benefits of keeping coupled:

- Simpler codebase (no abstraction overhead)
- Tighter type safety with concrete RaisinDB types
- Faster development velocity
- No maintenance burden of separate package

---

## Contributing

See [CONTRIBUTING.md](../../CONTRIBUTING.md) for development guidelines.
