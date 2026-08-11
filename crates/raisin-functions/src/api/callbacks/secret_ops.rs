// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Secret store callback type definitions.
//!
//! These carry PLAINTEXT across the seam (`secret_get` returns it,
//! `secret_put`/`secret_rotate` take it), so nothing here may ever be rendered
//! into a log line or an error string. The callbacks themselves do no policy
//! checking — [`SecretPolicy`](crate::types::SecretPolicy) is enforced one layer
//! up, in `api/raisindb/secrets.rs`, before any of these is invoked.

use raisin_error::Result;
use serde_json::Value;
use std::sync::Arc;

/// Read a secret's plaintext.
///
/// Args: `name`, optional pinned `version` ordinal (`None` = newest).
pub type SecretGetCallback = Arc<
    dyn Fn(
            String,      // name
            Option<u64>, // version ordinal
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>>
        + Send
        + Sync,
>;

/// Append a new version of a secret. Returns `{ name, version }`.
pub type SecretPutCallback = Arc<
    dyn Fn(
            String, // name
            String, // plaintext
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Metadata for the newest version of every secret in scope.
///
/// Never yields ciphertext or plaintext — the store's `SecretMetadata` type has
/// no field that could hold either.
pub type SecretListCallback = Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Value>>> + Send>>
        + Send
        + Sync,
>;

/// Append a new version stamped as a rotation. Returns `{ name, version }`.
pub type SecretRotateCallback = Arc<
    dyn Fn(
            String, // name
            String, // new plaintext
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Append a tombstone. Returns `{ name, version }` (the tombstone's ordinal).
pub type SecretDeleteCallback = Arc<
    dyn Fn(
            String, // name
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;
