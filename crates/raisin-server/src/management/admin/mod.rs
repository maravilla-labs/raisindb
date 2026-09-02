//! Operator endpoints for RaisinDB.
//!
//! Routes mounted under `/management/admin/*` and gated by the
//! `RAISIN_SUPERADMIN_TOKEN` env var. They exist for tenant provisioning,
//! credential recovery, and cross-tenant incident response — powers that
//! need to live outside the per-tenant scope.
//!
//! # Security model
//!
//! The mechanism is fully visible in this source tree; the secret value
//! is held by ops, the same way `RAISIN_MASTER_KEY` or any cloud root
//! credential is. To enable: set `RAISIN_SUPERADMIN_TOKEN` to a long
//! random string at server start. If the env var is unset (or empty),
//! these routes are not mounted at all — callers see 404, not 401. When
//! set, every request must carry `Authorization: Bearer <token>` and is
//! compared in constant time.
//!
//! Treat the token like a root credential. Rotation is by restart with
//! a new value — there is no rotation API.

#[cfg(feature = "storage-rocksdb")]
pub mod identity_users;
#[cfg(feature = "storage-rocksdb")]
pub mod jobs;
#[cfg(feature = "storage-rocksdb")]
pub mod passwords;
#[cfg(feature = "storage-rocksdb")]
pub mod scheduling;
#[cfg(feature = "storage-rocksdb")]
pub mod tenants;
