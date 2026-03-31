# raisin-audit

Audit logging interfaces and implementations for RaisinDB.

## Status

**Stub implementation** - This crate provides the core audit trait and an in-memory implementation for development/testing. Production RocksDB-backed implementation is not yet available.

## Components

- `AuditRepository` trait - Interface for audit log storage
- `InMemoryAuditRepo` - In-memory implementation (development/testing only)
- `make_log()` - Helper to create `AuditLog` entries

## Usage

```rust
use raisin_audit::{AuditRepository, InMemoryAuditRepo, make_log};
use raisin_models::nodes::audit_log::AuditLogAction;

let repo = InMemoryAuditRepo::default();

let log = make_log(
    "node-123".into(),
    "/docs/readme".into(),
    "main".into(),
    Some("user-456".into()),
    AuditLogAction::Update,
    None,
);

repo.write_log(log).await?;
```

## TODO

- [ ] RocksDB-backed `AuditRepository` implementation
- [ ] Query/filtering capabilities
- [ ] Retention policies
