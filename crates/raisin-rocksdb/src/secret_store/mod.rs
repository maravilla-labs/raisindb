// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! The secret store: the only place ciphertext lives, and the only place
//! plaintext is ever produced.
//!
//! A node property holds a `secret://name` reference ([`raisin_models::secret_ref`]);
//! the bytes behind it live here, in `cf::SECRETS`, sealed by [`raisin_crypto`].
//!
//! # Why this is a module in `raisin-rocksdb`, not a `raisin-secrets` crate
//!
//! The store must be callable from **three** places: the RocksDB write layer
//! (to vault a schema field as the node is written), `raisin-functions`, and the
//! HTTP transport. Both of the latter already depend on `raisin-rocksdb`.
//!
//! A separate crate would need a RocksDB handle, i.e. a dependency on
//! `raisin-rocksdb` — while `raisin-rocksdb`'s write path would need to depend
//! on *it*. That is a package cycle, and **Cargo rejects package cycles
//! regardless of features**; it is the same wall that forced the
//! `raisin-mcp` / `raisin-mcp-protocol` split (see CLAUDE.md, "MCP: both
//! directions"). The MCP split worked because the *wire types and client* had
//! no storage dependency to invert. Here the storage handle IS the thing, so
//! there is nothing to split out — except the reference syntax, which is
//! exactly what went to `raisin-models::secret_ref` so the SQL layer and the
//! transports can recognise a `secret://` value without pulling in the engine.
//!
//! # Ordering
//!
//! Versions are keyed by a **descending** HLC (see [`keys`]), so a forward
//! prefix scan is newest-first and `get` reads one entry. The human-facing
//! ordinal in [`SecretRecord::version`] is a label, not the ordering.
//!
//! # The two distinct absences
//!
//! `get` never collapses "deleted" and "not here yet" into one not-found:
//!
//! - [`SecretError::Gone`] — a tombstone exists. The secret was deleted.
//! - [`SecretError::Pending`] — no record for that version. On a replica this
//!   is indistinguishable from a convergence window, so it is reported as a
//!   *wait*, not a *no*. A caller that retries on `Pending` and gives up on
//!   `Gone` behaves correctly in both cases; one merged error would make that
//!   impossible.

pub(super) mod crud;
pub mod keys;
mod record;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use raisin_crypto::{CryptoError, SecretBox};
use raisin_hlc::NodeHLCState;
use raisin_models::secret_ref::SecretRefError;
use rocksdb::DB;

pub use record::{SecretMetadata, SecretOwner, SecretRecord};

/// Which `{tenant, repo, branch}` a secret operation addresses.
///
/// Branch is part of the storage key, so the same name on two branches is two
/// independent secrets. It is deliberately **not** part of the crypto context
/// (`SecretContext::node_secret` binds tenant and repo only) — a branch fork
/// copies the sealed bytes verbatim, and binding the branch would make every
/// copied secret unopenable on the fork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretScope {
    pub tenant: String,
    pub repo: String,
    pub branch: String,
}

impl SecretScope {
    pub fn new(
        tenant: impl Into<String>,
        repo: impl Into<String>,
        branch: impl Into<String>,
    ) -> Self {
        Self {
            tenant: tenant.into(),
            repo: repo.into(),
            branch: branch.into(),
        }
    }
}

/// Everything that can go wrong in the secret store.
///
/// Deliberately a dedicated enum rather than `raisin_error::Error`: the whole
/// value of this API is that its absences are distinguishable, and a
/// `NotFound(String)` cannot carry that. Converts into `raisin_error::Error`
/// for call sites that only need to propagate.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// The name is unusable (empty, NUL, whitespace).
    #[error("invalid secret name: {0}")]
    InvalidName(#[from] SecretRefError),

    /// A tombstone exists for this version. The secret was deleted; retrying
    /// will not help.
    #[error("secret '{name}' version {version} was deleted")]
    Gone { name: String, version: u64 },

    /// No record. Either it never existed, or this replica has not converged
    /// yet — the two are genuinely indistinguishable from here, so this is
    /// reported as a wait rather than a denial.
    #[error("secret '{name}'{} is not present on this node (never written, or not yet replicated)", version.map(|v| format!(" version {v}")).unwrap_or_default())]
    Pending { name: String, version: Option<u64> },

    /// The record names a master key this node does not hold. Reported before
    /// any decrypt attempt so the operator sees the missing id, not a tag
    /// mismatch that reads like corruption.
    #[error("secret '{name}' version {version} was sealed under master key id {key_id}, which this node's keyring does not hold (available: {available:?})")]
    UnknownKeyId {
        name: String,
        version: u64,
        key_id: u16,
        available: Vec<u16>,
    },

    /// `SecretBox::seal` produced a v1 (context-free) envelope.
    ///
    /// This store's crypto family is `V1Policy::Reject` — correct, because it
    /// is brand new and has no legacy data at rest — so a v1 blob written here
    /// could never be read back. `seal` emits v1 until `RAISIN_CRYPTO_EMIT_V2`
    /// is set, which is the deliberate rolling-upgrade gate for the *existing*
    /// families. Rather than silently writing unreadable bytes, or weakening
    /// this family to `Accept` and permanently forfeiting the context binding,
    /// the write refuses and says why.
    #[error(
        "refusing to write a v1 (context-free) secret: the node secret family rejects v1 on read, \
         so this value could never be read back. Set RAISIN_CRYPTO_EMIT_V2 on every node before \
         enabling the secret store."
    )]
    V2EmissionRequired,

    #[error("crypto: {0}")]
    Crypto(#[from] CryptoError),

    #[error("secret store column family '{0}' is not open")]
    ColumnFamilyMissing(&'static str),

    #[error("secret store storage error: {0}")]
    Storage(String),

    #[error("secret store encoding error: {0}")]
    Encoding(String),
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, SecretError>;

impl From<SecretError> for raisin_error::Error {
    fn from(e: SecretError) -> Self {
        match e {
            SecretError::InvalidName(_) => raisin_error::Error::Validation(e.to_string()),
            SecretError::Gone { .. } => raisin_error::Error::NotFound(e.to_string()),
            SecretError::Pending { .. } => raisin_error::Error::NotFound(e.to_string()),
            SecretError::UnknownKeyId { .. } => raisin_error::Error::InvalidState(e.to_string()),
            SecretError::V2EmissionRequired => raisin_error::Error::InvalidState(e.to_string()),
            SecretError::Crypto(_) => raisin_error::Error::Internal(e.to_string()),
            SecretError::ColumnFamilyMissing(_) => raisin_error::Error::Backend(e.to_string()),
            SecretError::Storage(_) => raisin_error::Error::Backend(e.to_string()),
            SecretError::Encoding(_) => raisin_error::Error::Encoding(e.to_string()),
        }
    }
}

/// Append-only, versioned, encrypted key/value store scoped to a branch.
#[derive(Clone)]
pub struct SecretStore {
    pub(super) db: Arc<DB>,
    pub(super) secret_box: Arc<SecretBox>,
    /// Minted per write. Must be a *ticking* state, not `HLC::now()`: `now()`
    /// pins the counter at 0, so two writes in the same millisecond would
    /// produce the same key and the second would OVERWRITE the first — the one
    /// thing an append-only store must never do.
    pub(super) hlc: Arc<NodeHLCState>,
}

impl SecretStore {
    /// Build a store with its own HLC state.
    ///
    /// `node_id` is the cluster node id, which seeds the HLC state exactly as
    /// [`crate::repositories::revisions`] does.
    pub fn new(db: Arc<DB>, secret_box: Arc<SecretBox>, node_id: impl Into<String>) -> Self {
        Self {
            db,
            secret_box,
            hlc: Arc::new(NodeHLCState::new(node_id.into())),
        }
    }

    /// Build a store sharing an existing HLC state — preferred inside the
    /// server, so secret revisions interleave monotonically with node
    /// revisions minted by the same process.
    pub fn with_hlc(db: Arc<DB>, secret_box: Arc<SecretBox>, hlc: Arc<NodeHLCState>) -> Self {
        Self {
            db,
            secret_box,
            hlc,
        }
    }

    /// The keyring backing this store, for diagnostics.
    pub fn secret_box(&self) -> &Arc<SecretBox> {
        &self.secret_box
    }
}
