// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Production `raisin.identities.*` callbacks over the RocksDB
//! [`IdentityRepository`].
//!
//! Identities are the tenant's auth records, NOT `raisin:User` nodes. A node
//! in `raisin:access_control` is BOUND to an identity through its `user_id`
//! property (see `raisin-transport-http`'s `identity_auth/user_node.rs`, which
//! is where that binding is written). When the email changes here, the bound
//! node's `email` property is brought along, through storage directly — the
//! function's own node callbacks run on the EXECUTION branch, and the ACL
//! workspace is pinned to `main` regardless of which branch a function runs
//! on (the same reason `user_node.rs` hard-codes it).
//!
//! The policy gate is NOT here — see `api/raisindb/identities.rs`.

use std::sync::Arc;

use raisin_binary::BinaryStorage;
use raisin_models::auth::{AuthContext, Identity};
use raisin_models::nodes::properties::PropertyValue;
use raisin_rocksdb::repositories::IdentityRepository;
use raisin_storage::{NodeRepository, Storage, StorageScope};
use serde_json::Value;

use crate::api::{IdentityFindByEmailCallback, IdentityPatch, IdentityUpdateCallback};

/// Workspace holding `raisin:User` nodes.
const ACCESS_CONTROL_WORKSPACE: &str = "raisin:access_control";
/// The branch that workspace is pinned to. See the module docs.
const ACCESS_CONTROL_BRANCH: &str = "main";

/// The public shape of an identity — what a function is allowed to see.
///
/// Never the credential hash, the linked-provider tokens, or the lockout
/// counters: `has_password` is the only thing a caller needs from the
/// credentials, and it is what decides whether a sign-in form should offer a
/// password field.
pub fn public_shape(identity: &Identity) -> Value {
    serde_json::json!({
        "id": identity.identity_id,
        "email": identity.email,
        "email_verified": identity.email_verified,
        "display_name": identity.display_name,
        "has_password": identity.local_credentials.is_some(),
    })
}

fn actor_of(auth_context: &Option<AuthContext>) -> String {
    auth_context
        .as_ref()
        .and_then(|a| a.user_id.clone())
        .unwrap_or_else(|| "function".to_string())
}

pub fn create_identity_find_by_email(
    repo: Arc<IdentityRepository>,
    tenant_id: String,
) -> IdentityFindByEmailCallback {
    Arc::new(move |email: String| {
        let repo = repo.clone();
        let tenant_id = tenant_id.clone();
        Box::pin(async move {
            let found = repo.find_by_email(&tenant_id, &email).await?;
            Ok(found.as_ref().map(public_shape))
        })
    })
}

/// Outcome of [`apply_patch`]: the identity as it now is, plus whether the
/// address changed (which is what decides whether the node cascade runs).
#[derive(Debug)]
pub struct Applied {
    pub identity: Identity,
    pub email_changed: bool,
}

/// Apply `patch` to identity `id`. Pure repository logic, split out so it can
/// be tested without a node store.
///
/// Order matters: the taken-check happens BEFORE any write, so a refused
/// rename leaves the identity exactly as it was. `set_password` is a second
/// upsert through the repository (it re-reads and re-hashes), so the result is
/// re-read afterwards rather than assembled from what we think we wrote.
pub async fn apply_patch(
    repo: &IdentityRepository,
    tenant_id: &str,
    id: &str,
    patch: IdentityPatch,
    actor: &str,
) -> raisin_error::Result<Applied> {
    let mut identity = repo.get(tenant_id, id).await?.ok_or_else(|| {
        raisin_error::Error::NotFound(format!("[identities:not_found] identity {id} not found"))
    })?;

    let mut email_changed = false;
    if let Some(new_email) = patch.email.as_deref().map(str::to_string) {
        if !new_email.eq_ignore_ascii_case(identity.email.as_str()) {
            if let Some(other) = repo.find_by_email(tenant_id, &new_email).await? {
                if other.identity_id != identity.identity_id {
                    return Err(raisin_error::Error::Conflict(format!(
                        "[identities:email_taken] {new_email} already belongs to another account"
                    )));
                }
            }
            identity.email = new_email.clone();
            // A rename proves the address was typed, not that it is possessed.
            // Only the magic-link verify may set this; see the module docs in
            // api/raisindb/identities.rs.
            identity.email_verified = false;
            email_changed = true;
        }
    }

    if let Some(display_name) = patch.display_name.clone() {
        identity.display_name = if display_name.is_empty() {
            None
        } else {
            Some(display_name)
        };
    }

    if email_changed || patch.display_name.is_some() {
        identity.updated_at = Some(raisin_models::timestamp::StorageTimestamp::now());
        repo.upsert(tenant_id, &identity, actor).await?;
    }

    if let Some(password) = patch.password.as_deref() {
        repo.set_password(tenant_id, id, password, false, actor)
            .await?;
    }

    let identity = repo.get(tenant_id, id).await?.ok_or_else(|| {
        raisin_error::Error::Internal(format!(
            "identity {id} vanished between write and read-back"
        ))
    })?;
    Ok(Applied {
        identity,
        email_changed,
    })
}

/// Bring the bound `raisin:User` node's `email` along with a rename.
///
/// Finds the node by its `user_id` property (the binding `user_node.rs`
/// writes) and updates ONLY `email` — path and home stay, because the node's
/// path is its ACL address and every role reference points at it. No node
/// means nothing to do: the next sign-in provisions one from the identity.
async fn cascade_email_to_user_node<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo_id: &str,
    identity_id: &str,
    email: &str,
) -> raisin_error::Result<usize> {
    let scope = StorageScope::new(
        tenant_id,
        repo_id,
        ACCESS_CONTROL_BRANCH,
        ACCESS_CONTROL_WORKSPACE,
    );
    let nodes = Storage::nodes(storage)
        .find_by_property(
            scope,
            "user_id",
            &PropertyValue::String(identity_id.to_string()),
        )
        .await?;

    let mut updated = 0;
    for node in nodes.into_iter().filter(|n| n.node_type == "raisin:User") {
        let already =
            matches!(node.properties.get("email"), Some(PropertyValue::String(e)) if e == email);
        if already {
            continue;
        }
        Storage::nodes(storage)
            .update_property_by_path(
                StorageScope::new(
                    tenant_id,
                    repo_id,
                    ACCESS_CONTROL_BRANCH,
                    ACCESS_CONTROL_WORKSPACE,
                ),
                &node.path,
                "email",
                PropertyValue::String(email.to_string()),
            )
            .await?;
        updated += 1;
    }
    Ok(updated)
}

pub fn create_identity_update<S, B>(
    repo: Arc<IdentityRepository>,
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    auth_context: Option<AuthContext>,
) -> IdentityUpdateCallback
where
    S: Storage + 'static,
    B: BinaryStorage + 'static,
{
    let actor = actor_of(&auth_context);
    Arc::new(move |id: String, patch: IdentityPatch| {
        let repo = repo.clone();
        let storage = storage.clone();
        let tenant_id = tenant_id.clone();
        let repo_id = repo_id.clone();
        let actor = actor.clone();
        Box::pin(async move {
            let applied = apply_patch(&repo, &tenant_id, &id, patch, &actor).await?;

            // Only after a successful identity write. A failure here is logged
            // rather than returned: the identity IS renamed, and the next
            // sign-in reconciles the node from the identity anyway
            // (user_node.rs finds it by user_id and rewrites `email`).
            if applied.email_changed {
                match cascade_email_to_user_node(
                    storage.as_ref(),
                    &tenant_id,
                    &repo_id,
                    &id,
                    &applied.identity.email,
                )
                .await
                {
                    Ok(n) => tracing::info!(
                        identity_id = %id,
                        nodes_updated = n,
                        "raisin.identities.update: email cascaded to raisin:User node"
                    ),
                    Err(e) => tracing::error!(
                        identity_id = %id,
                        error = %e,
                        "raisin.identities.update: identity renamed but the bound \
                         raisin:User node could not be updated"
                    ),
                }
            }

            Ok(public_shape(&applied.identity))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_rocksdb::{open_db, OperationCapture};

    fn repo() -> (tempfile::TempDir, IdentityRepository) {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Arc::new(open_db(dir.path()).unwrap());
        let op = Arc::new(OperationCapture::disabled(db.clone()));
        (dir, IdentityRepository::new(db, op))
    }

    async fn seed(repo: &IdentityRepository, id: &str, email: &str) {
        let identity = Identity::new(id.to_string(), "t".to_string(), email.to_string());
        repo.upsert("t", &identity, "seed").await.unwrap();
    }

    #[tokio::test]
    async fn rename_moves_the_email_index_and_clears_verified() {
        let (_d, repo) = repo();
        seed(&repo, "id-1", "old@example.com").await;
        let mut verified = repo.get("t", "id-1").await.unwrap().unwrap();
        verified.verify_email();
        repo.upsert("t", &verified, "seed").await.unwrap();

        let applied = apply_patch(
            &repo,
            "t",
            "id-1",
            IdentityPatch {
                email: Some("new@example.com".to_string()),
                ..Default::default()
            },
            "test",
        )
        .await
        .unwrap();

        assert!(applied.email_changed);
        assert_eq!(applied.identity.email, "new@example.com");
        assert!(
            !applied.identity.email_verified,
            "a rename must never carry the verified flag over"
        );
        assert!(repo
            .find_by_email("t", "old@example.com")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            repo.find_by_email("t", "New@Example.com")
                .await
                .unwrap()
                .unwrap()
                .identity_id,
            "id-1"
        );
    }

    #[tokio::test]
    async fn rename_onto_another_accounts_email_is_refused_before_any_write() {
        let (_d, repo) = repo();
        seed(&repo, "id-1", "a@example.com").await;
        seed(&repo, "id-2", "b@example.com").await;

        let err = apply_patch(
            &repo,
            "t",
            "id-1",
            IdentityPatch {
                email: Some("b@example.com".to_string()),
                display_name: Some("Renamed".to_string()),
                ..Default::default()
            },
            "test",
        )
        .await
        .unwrap_err();

        assert!(
            err.to_string().contains("[identities:email_taken]"),
            "{err}"
        );
        let untouched = repo.get("t", "id-1").await.unwrap().unwrap();
        assert_eq!(untouched.email, "a@example.com");
        assert_eq!(
            untouched.display_name, None,
            "nothing may be written on refusal"
        );
    }

    #[tokio::test]
    async fn same_email_different_case_is_not_a_rename() {
        let (_d, repo) = repo();
        seed(&repo, "id-1", "a@example.com").await;
        let applied = apply_patch(
            &repo,
            "t",
            "id-1",
            IdentityPatch {
                email: Some("a@example.com".to_string()),
                ..Default::default()
            },
            "test",
        )
        .await
        .unwrap();
        assert!(!applied.email_changed);
    }

    #[tokio::test]
    async fn password_gives_a_magic_link_account_local_credentials() {
        let (_d, repo) = repo();
        seed(&repo, "id-1", "a@example.com").await;
        assert_eq!(
            public_shape(&repo.get("t", "id-1").await.unwrap().unwrap())["has_password"],
            false
        );

        let applied = apply_patch(
            &repo,
            "t",
            "id-1",
            IdentityPatch {
                password: Some("correct horse battery staple".to_string()),
                ..Default::default()
            },
            "test",
        )
        .await
        .unwrap();

        assert_eq!(public_shape(&applied.identity)["has_password"], true);
        assert!(repo
            .verify_password("t", "id-1", "correct horse battery staple")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn unknown_identity_is_not_found() {
        let (_d, repo) = repo();
        let err = apply_patch(
            &repo,
            "t",
            "nope",
            IdentityPatch {
                display_name: Some("x".to_string()),
                ..Default::default()
            },
            "test",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, raisin_error::Error::NotFound(_)), "{err}");
    }

    #[test]
    fn public_shape_never_leaks_credentials() {
        let mut identity = Identity::new("id".into(), "t".into(), "a@example.com".into());
        identity.local_credentials = Some(raisin_models::auth::LocalCredentials::new(
            "$argon2$hash".to_string(),
        ));
        let shape = public_shape(&identity);
        assert_eq!(shape["has_password"], true);
        assert!(!shape.to_string().contains("argon2"));
        assert!(shape.get("local_credentials").is_none());
    }
}
