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

//! Resolving a `secret://` reference to the plaintext behind it.
//!
//! Without this, the store is write-only: a vaulted field stores a reference
//! and nothing turns that reference back into a credential. This module is the
//! **use** side.
//!
//! # This must never be reachable from a read path that serves clients
//!
//! A node read — REST, WebSocket, SQL `SELECT`, a subscription event, the
//! console — returns the **reference**, verbatim, exactly as stored. There is
//! no "resolve on read" mode, no query flag, and deliberately no HTTP endpoint
//! that returns a value (see `raisin-transport-http`'s `handlers/secrets.rs`,
//! whose route table has no read-the-value entry at all).
//!
//! Resolution is for server-side **use**: the outbound HTTP client about to
//! sign a request, the IMAP adapter about to authenticate, a flow step about to
//! call a provider. Each of those consumes the plaintext and never puts it in a
//! response body. If you find yourself calling anything in this module while
//! building a payload that will be serialized to a client, that is the bug.
//!
//! # The two absences are preserved, not collapsed
//!
//! [`SecretError::Gone`] (a tombstone exists — the secret was deleted, retrying
//! is pointless) and [`SecretError::Pending`] (no record — never written, or
//! this replica has not converged) stay distinct all the way out to the caller,
//! because a caller that retries on `Pending` and gives up on `Gone` is correct
//! in both cases and a merged not-found makes that impossible. Nothing here
//! flattens them into an `Option`.

use raisin_models::secret_ref::SecretRef;

use super::{Result, SecretError, SecretScope, SecretStore};

impl SecretStore {
    /// Resolve a parsed reference to its plaintext bytes.
    ///
    /// `ref.version = None` reads the newest version; a pinned version reads
    /// exactly that one, which is why an older node revision keeps resolving
    /// after a rotation.
    ///
    /// Errors are the store's, unchanged: [`SecretError::Gone`] for a
    /// tombstone, [`SecretError::Pending`] for an absent record,
    /// [`SecretError::UnknownKeyId`] for a keyring gap.
    pub fn resolve(&self, scope: &SecretScope, reference: &SecretRef) -> Result<Vec<u8>> {
        self.get(scope, &reference.name, reference.version)
    }

    /// [`SecretStore::resolve`], as UTF-8.
    ///
    /// Nearly every consumer (an API key, a password, a bearer token) wants a
    /// string. A secret whose bytes are not UTF-8 is an
    /// [`SecretError::Encoding`] here rather than a lossy replacement — a
    /// mangled credential fails at the provider with an error that explains
    /// nothing.
    pub fn resolve_string(&self, scope: &SecretScope, reference: &SecretRef) -> Result<String> {
        let bytes = self.resolve(scope, reference)?;
        String::from_utf8(bytes).map_err(|_| {
            SecretError::Encoding(format!(
                "secret '{}' is not valid UTF-8; use resolve() for raw bytes",
                reference.name
            ))
        })
    }

    /// Resolve `value` if — and only if — it is a `secret://` reference.
    ///
    /// The convenience for a caller holding a property value that *may* be a
    /// reference: a connector config field that is a literal password on one
    /// deployment and a vaulted reference on another can be consumed the same
    /// way.
    ///
    /// - `Ok(None)` — `value` is not a reference. The caller keeps its literal.
    /// - `Ok(Some(plaintext))` — it was a reference and it resolved.
    /// - `Err(..)` — it *was* a reference and resolving it failed, with the
    ///   `Gone` / `Pending` split intact.
    ///
    /// # Why this returns `Result<Option<_>>` and not `Option<_>`
    ///
    /// A bare `Option` has one `None` for two opposite situations: "this was a
    /// plain string, use it as-is" and "this was a reference that could not be
    /// resolved". Under that signature a deleted secret degrades into the
    /// caller sending the literal text `secret://stripe_key` to a payment
    /// provider as an API key. The three-way return makes that unrepresentable.
    pub fn resolve_if_reference(&self, scope: &SecretScope, value: &str) -> Result<Option<String>> {
        if !SecretRef::is_secret_ref(value) {
            return Ok(None);
        }
        let reference = SecretRef::parse(value)?;
        self.resolve_string(scope, &reference).map(Some)
    }

    /// [`SecretStore::resolve_if_reference`] for raw bytes.
    pub fn resolve_bytes_if_reference(
        &self,
        scope: &SecretScope,
        value: &str,
    ) -> Result<Option<Vec<u8>>> {
        if !SecretRef::is_secret_ref(value) {
            return Ok(None);
        }
        let reference = SecretRef::parse(value)?;
        self.resolve(scope, &reference).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Once};

    use raisin_crypto::{Keyring, SecretBox};
    use rocksdb::{Options, DB};
    use tempfile::TempDir;

    use super::*;
    use crate::cf;
    use crate::secret_store::SecretOwner;

    /// See `secret_store::tests`: the node-secret family is `V1Policy::Reject`,
    /// so a write refuses unless v2 emission is on. Only ever set, never
    /// cleared, so parallel tests cannot race on it.
    static EMIT_V2: Once = Once::new();

    fn test_store() -> (TempDir, SecretStore) {
        EMIT_V2.call_once(|| std::env::set_var("RAISIN_CRYPTO_EMIT_V2", "1"));
        let dir = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = DB::open_cf(&opts, dir.path(), vec![cf::SECRETS]).unwrap();
        let keys = Arc::new(Keyring::new(vec![(1, [1u8; 32])], 1).unwrap());
        let store = SecretStore::new(
            Arc::new(db),
            Arc::new(SecretBox::with_keyring(keys)),
            "test-node",
        );
        (dir, store)
    }

    fn scope() -> SecretScope {
        SecretScope::new("acme", "site", "main")
    }

    #[test]
    fn resolves_a_latest_reference() {
        let (_d, store) = test_store();
        store
            .put(
                &scope(),
                "stripe_key",
                b"sk_live_abc",
                &SecretOwner::actor("alice"),
            )
            .unwrap();

        let r = SecretRef::parse("secret://stripe_key").unwrap();
        assert_eq!(store.resolve_string(&scope(), &r).unwrap(), "sk_live_abc");
    }

    /// A pinned reference must keep resolving after a rotation — that is the
    /// whole reason rotation appends instead of replacing.
    #[test]
    fn a_pinned_reference_survives_rotation() {
        let (_d, store) = test_store();
        let owner = SecretOwner::actor("alice");
        store.put(&scope(), "k", b"v1", &owner).unwrap();
        store.rotate(&scope(), "k", b"v2", &owner).unwrap();

        let pinned = SecretRef::parse("secret://k@1").unwrap();
        let latest = SecretRef::parse("secret://k").unwrap();
        assert_eq!(store.resolve_string(&scope(), &pinned).unwrap(), "v1");
        assert_eq!(store.resolve_string(&scope(), &latest).unwrap(), "v2");
    }

    /// THE invariant of this module: deleted and not-yet-here are different
    /// errors. A caller retries on one and gives up on the other.
    #[test]
    fn gone_and_pending_stay_distinguishable() {
        let (_d, store) = test_store();
        store
            .put(&scope(), "k", b"v", &SecretOwner::actor("alice"))
            .unwrap();
        store
            .delete(&scope(), "k", &SecretOwner::actor("alice"))
            .unwrap();

        let deleted = SecretRef::parse("secret://k").unwrap();
        match store.resolve(&scope(), &deleted) {
            Err(SecretError::Gone { name, .. }) => assert_eq!(name, "k"),
            other => panic!("expected Gone for a tombstoned secret, got {other:?}"),
        }

        let absent = SecretRef::parse("secret://never_written").unwrap();
        match store.resolve(&scope(), &absent) {
            Err(SecretError::Pending { name, .. }) => assert_eq!(name, "never_written"),
            other => panic!("expected Pending for an absent secret, got {other:?}"),
        }
    }

    /// A non-reference is `Ok(None)` — "use your literal" — and is NOT confused
    /// with a reference that failed to resolve.
    #[test]
    fn resolve_if_reference_distinguishes_literal_from_failure() {
        let (_d, store) = test_store();
        store
            .put(&scope(), "k", b"v", &SecretOwner::actor("alice"))
            .unwrap();

        assert_eq!(
            store
                .resolve_if_reference(&scope(), "a-literal-password")
                .unwrap(),
            None
        );
        assert_eq!(
            store.resolve_if_reference(&scope(), "secret://k").unwrap(),
            Some("v".to_string())
        );
        // A reference that cannot resolve is an ERROR, never `None` — under an
        // `Option` return the caller would send the text `secret://missing` to
        // a provider as a credential.
        assert!(matches!(
            store.resolve_if_reference(&scope(), "secret://missing"),
            Err(SecretError::Pending { .. })
        ));
    }

    /// A name with slashes — the auto-vault convention — resolves like any
    /// other.
    #[test]
    fn resolves_a_node_field_name() {
        let (_d, store) = test_store();
        let name = raisin_models::secret_ref::node_field_secret_name("01H8XY", "venue.token");
        store
            .put(&scope(), &name, b"tok", &SecretOwner::actor("alice"))
            .unwrap();

        let r = SecretRef::parse(&format!("secret://{name}")).unwrap();
        assert_eq!(store.resolve_string(&scope(), &r).unwrap(), "tok");
    }

    /// The scope is part of the key, so another branch is `Pending`, not a
    /// cross-branch read.
    #[test]
    fn another_branch_does_not_see_the_secret() {
        let (_d, store) = test_store();
        store
            .put(&scope(), "k", b"v", &SecretOwner::actor("alice"))
            .unwrap();

        let other = SecretScope::new("acme", "site", "feature");
        let r = SecretRef::parse("secret://k").unwrap();
        assert!(matches!(
            store.resolve(&other, &r),
            Err(SecretError::Pending { .. })
        ));
    }
}
