// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tenant-identity callback type definitions.
//!
//! These reach the auth identities stored in RocksDB (`IdentityRepository`),
//! NOT `raisin:User` nodes. The callbacks do no policy checking —
//! [`IdentityPolicy`](crate::types::IdentityPolicy) is enforced one layer up,
//! in `api/raisindb/identities.rs`, before any of these is invoked.
//!
//! An [`IdentityPatch`] may carry a PASSWORD across the seam, so nothing here
//! may ever be rendered into a log line or an error string; its `Debug` impl
//! redacts it for that reason.

use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// The fields `raisin.identities.update` may change.
///
/// `email_verified` is deliberately absent: a rename proves the caller can
/// TYPE an address, not that the account holder POSSESSES it. Only the
/// magic-link verify step proves possession, so only it may set the flag.
#[derive(Clone, Default)]
pub struct IdentityPatch {
    /// New primary address. Already normalised (trimmed, lowercased) by the
    /// API layer; the callback must still treat it as the source of truth.
    pub email: Option<String>,
    /// New local password (plaintext; hashed at the repository).
    pub password: Option<String>,
    /// New display name.
    pub display_name: Option<String>,
}

impl IdentityPatch {
    /// Whether the patch changes anything at all.
    pub fn is_empty(&self) -> bool {
        self.email.is_none() && self.password.is_none() && self.display_name.is_none()
    }
}

impl std::fmt::Debug for IdentityPatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityPatch")
            .field("email", &self.email)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("display_name", &self.display_name)
            .finish()
    }
}

/// Look up an identity by (already normalised) email.
///
/// Resolves to the public identity shape
/// `{ id, email, email_verified, display_name, has_password }`, or `None`.
pub type IdentityFindByEmailCallback = Arc<
    dyn Fn(
            String, // email
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<Value>>> + Send>>
        + Send
        + Sync,
>;

/// Apply a patch to an identity and resolve to its new public shape.
///
/// Fails with a message containing `[identities:email_taken]` when the new
/// address belongs to a different identity.
pub type IdentityUpdateCallback = Arc<
    dyn Fn(
            String,        // identity id
            IdentityPatch, // what to change
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;
