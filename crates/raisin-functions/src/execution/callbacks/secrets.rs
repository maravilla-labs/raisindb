// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Production callback factories for the secret store.
//!
//! Each factory captures the shared [`SecretStore`] plus the execution's
//! `{tenant, repo, branch}` [`SecretScope`], so a function can only ever address
//! secrets in its own branch — the scope is supplied by the host, never by the
//! caller. Which NAMES within that scope are reachable is the separate question
//! the function's `SecretPolicy` answers, one layer up in
//! `api/raisindb/secrets.rs`.
//!
//! # `spawn_blocking`, not a bare call
//!
//! `SecretStore`'s operations are synchronous RocksDB reads and writes. Calling
//! them straight from an async task parks a runtime worker on a blocking write —
//! the exact shape that froze the production job queue (see the job-pool
//! deadlock in the repo history). Every operation here therefore goes through
//! `spawn_blocking`.

use std::sync::Arc;

use raisin_models::auth::AuthContext;
use raisin_rocksdb::secret_store::{SecretOwner, SecretScope, SecretStore};
use serde_json::Value;

use crate::api::{
    SecretDeleteCallback, SecretGetCallback, SecretListCallback, SecretPutCallback,
    SecretRotateCallback,
};

/// Everything the callbacks need, captured once per execution.
#[derive(Clone)]
struct Ctx {
    store: Arc<SecretStore>,
    scope: SecretScope,
    actor: Option<String>,
}

impl Ctx {
    fn owner(&self) -> SecretOwner {
        SecretOwner {
            actor: self.actor.clone(),
            node: None,
            field: None,
        }
    }
}

fn ctx(
    store: Arc<SecretStore>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> Ctx {
    Ctx {
        store,
        scope: SecretScope::new(tenant_id, repo_id, branch),
        // `None` becomes "anonymous" at the store, matching how the node write
        // layer stamps `created_by`.
        actor: auth_context.and_then(|a| a.user_id.clone()),
    }
}

/// Run a blocking store operation off the async worker threads.
async fn blocking<T, F>(f: F) -> raisin_error::Result<T>
where
    F: FnOnce() -> raisin_error::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|e| {
        raisin_error::Error::Internal(format!("secret store task failed to complete: {e}"))
    })?
}

pub fn create_secret_get(
    store: Arc<SecretStore>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> SecretGetCallback {
    let c = ctx(store, tenant_id, repo_id, branch, auth_context);
    Arc::new(move |name: String, version: Option<u64>| {
        let c = c.clone();
        Box::pin(async move {
            blocking(move || {
                let bytes = c.store.get(&c.scope, &name, version)?;
                // Secrets are stored as opaque bytes; the binding's contract is
                // text. Invalid UTF-8 is reported WITHOUT the bytes — a lossy
                // render would put fragments of the secret in an error string.
                String::from_utf8(bytes).map_err(|_| {
                    raisin_error::Error::Validation(format!(
                        "secret '{name}' is not valid UTF-8 and cannot be read as text"
                    ))
                })
            })
            .await
        })
    })
}

pub fn create_secret_put(
    store: Arc<SecretStore>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> SecretPutCallback {
    let c = ctx(store, tenant_id, repo_id, branch, auth_context);
    Arc::new(move |name: String, plaintext: String| {
        let c = c.clone();
        Box::pin(async move {
            blocking(move || {
                let (name, version) =
                    c.store
                        .put(&c.scope, &name, plaintext.as_bytes(), &c.owner())?;
                Ok::<Value, raisin_error::Error>(
                    serde_json::json!({ "name": name, "version": version }),
                )
            })
            .await
        })
    })
}

pub fn create_secret_list(
    store: Arc<SecretStore>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> SecretListCallback {
    let c = ctx(store, tenant_id, repo_id, branch, auth_context);
    Arc::new(move || {
        let c = c.clone();
        Box::pin(async move {
            blocking(move || {
                let metadata = c.store.list(&c.scope)?;
                // SecretMetadata has no field that could hold ciphertext or
                // plaintext, so serializing the whole thing is safe by type.
                metadata
                    .into_iter()
                    .map(|m| {
                        serde_json::to_value(m).map_err(|e| {
                            raisin_error::Error::Encoding(format!("serialize secret metadata: {e}"))
                        })
                    })
                    .collect::<raisin_error::Result<Vec<Value>>>()
            })
            .await
        })
    })
}

pub fn create_secret_rotate(
    store: Arc<SecretStore>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> SecretRotateCallback {
    let c = ctx(store, tenant_id, repo_id, branch, auth_context);
    Arc::new(move |name: String, plaintext: String| {
        let c = c.clone();
        Box::pin(async move {
            blocking(move || {
                let (name, version) =
                    c.store
                        .rotate(&c.scope, &name, plaintext.as_bytes(), &c.owner())?;
                Ok::<Value, raisin_error::Error>(
                    serde_json::json!({ "name": name, "version": version }),
                )
            })
            .await
        })
    })
}

pub fn create_secret_delete(
    store: Arc<SecretStore>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> SecretDeleteCallback {
    let c = ctx(store, tenant_id, repo_id, branch, auth_context);
    Arc::new(move |name: String| {
        let c = c.clone();
        Box::pin(async move {
            blocking(move || {
                let version = c.store.delete(&c.scope, &name, &c.owner())?;
                Ok::<Value, raisin_error::Error>(
                    serde_json::json!({ "name": name, "version": version }),
                )
            })
            .await
        })
    })
}
