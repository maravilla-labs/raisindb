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

//! Secrets across the node lifecycle: promote, copy, delete, clear, fork.
//!
//! Every test here turns on the same asymmetry. A `secret://` reference is
//! branch-agnostic and embeds the NODE ID, while the store is branch-scoped.
//! So an operation that moves a node has to ask two questions — did the branch
//! change, and did the id change — and the answers point in opposite
//! directions:
//!
//! - **promotion** preserves the id and changes the branch → COPY the sealed
//!   record across;
//! - **copy** keeps the branch and mints a new id → RE-VAULT under the new name.
//!
//! Getting either backwards produces a node that looks perfectly healthy, because
//! reads return the reference verbatim and never resolve it. That is what makes
//! these regression tests rather than unit tests: the failure is invisible from
//! the node API.

use std::collections::HashMap;
use std::sync::{Arc, Once};

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_crypto::{Keyring, SecretBox};
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::secret_ref::SecretRef;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::secret_store::{SecretError, SecretScope, SecretStore};
use raisin_rocksdb::{fractional_index, RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalContext;
use raisin_storage::{
    BranchRepository, CommitMetadata, DeleteNodeOptions, NodeRepository, NodeTypeRepository,
    RegistryRepository, RepositoryManagementRepository, Storage, StorageScope, Transaction,
};
use tempfile::TempDir;

const REPO: &str = "repo";
const MAIN: &str = "main";
const WS: &str = "content";

/// The node-secret crypto family rejects v1 envelopes on read, so the store
/// refuses to WRITE one. Only ever set, never cleared, so parallel tests in this
/// binary cannot race on it.
static EMIT_V2: Once = Once::new();

fn enable_v2_emission() {
    EMIT_V2.call_once(|| std::env::set_var("RAISIN_CRYPTO_EMIT_V2", "1"));
}

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
    tenant: String,
}

impl Env {
    async fn new(tenant: &str) -> Result<Self> {
        enable_v2_emission();

        let dir = TempDir::new().unwrap();
        let mut config = RocksDBConfig::default();
        config.path = dir.path().to_path_buf();
        let storage = Arc::new(RocksDBStorage::with_config(config)?);

        let keys = Arc::new(Keyring::new(vec![(1, [9u8; 32])], 1).unwrap());
        assert!(
            storage.set_secret_store(Arc::new(SecretStore::new(
                storage.db().clone(),
                Arc::new(SecretBox::with_keyring(keys)),
                "secret-lifecycle-node",
            ))),
            "the fixture must own the secret store"
        );

        storage
            .registry()
            .register_tenant(tenant, HashMap::new())
            .await?;
        storage
            .repository_management()
            .create_repository(
                tenant,
                REPO,
                RepositoryConfig {
                    default_language: "en".to_string(),
                    supported_languages: vec!["en".to_string()],
                    locale_fallback_chains: HashMap::new(),
                    default_branch: MAIN.to_string(),
                    description: None,
                    tags: HashMap::new(),
                },
            )
            .await?;
        storage
            .branches()
            .create_branch(tenant, REPO, MAIN, "system", None, None, false, false)
            .await?;

        let mut workspace = Workspace::new(WS.to_string());
        workspace.config.default_branch = MAIN.to_string();
        WorkspaceService::new(storage.clone())
            .put(tenant, REPO, workspace)
            .await?;

        Ok(Self {
            _dir: dir,
            storage,
            tenant: tenant.to_string(),
        })
    }

    /// A node type with TWO encrypted fields, so "tombstone the cleared one"
    /// can be told apart from "tombstone everything".
    async fn seed_node_type(&self, name: &str) -> Result<()> {
        self.storage
            .node_types()
            .create(
                BranchScope::new(&self.tenant, REPO, MAIN),
                serde_json::from_value(serde_json::json!({
                    "name": name,
                    "allowed_children": ["*"],
                    "properties": [
                        { "name": "host", "type": "String" },
                        { "name": "password", "type": "String", "encrypted": true },
                        { "name": "token", "type": "String", "encrypted": true },
                    ],
                }))
                .map(|nt: NodeType| nt)
                .expect("node type fixture must deserialize"),
                CommitMetadata::system("seed node type"),
            )
            .await?;
        Ok(())
    }

    async fn fork(&self, from: &str, to: &str) -> Result<()> {
        self.storage
            .branches()
            .create_branch(
                &self.tenant,
                REPO,
                to,
                "tester",
                None,
                Some(from.to_string()),
                false,
                false,
            )
            .await
            .map(|_| ())
    }

    /// An EMPTY branch — no fork, so nothing was copied and a promotion onto it
    /// has to carry everything itself. This is the shape that exposed the bug.
    async fn empty_branch(&self, name: &str) -> Result<()> {
        self.storage
            .branches()
            .create_branch(&self.tenant, REPO, name, "tester", None, None, false, false)
            .await
            .map(|_| ())
    }

    fn scope<'a>(&'a self, branch: &'a str) -> StorageScope<'a> {
        StorageScope::new(&self.tenant, REPO, branch, WS)
    }

    fn secrets(&self, branch: &str) -> SecretScope {
        SecretScope::new(&self.tenant, REPO, branch)
    }

    fn store(&self) -> Arc<SecretStore> {
        self.storage.secret_store().expect("store must build")
    }

    /// Write one node through the transaction path (the vaulting path) and commit.
    async fn write(&self, branch: &str, node: &Node) -> Result<()> {
        let tx = self.storage.begin().await?;
        tx.set_tenant_repo(&self.tenant, REPO)?;
        tx.set_branch(branch)?;
        tx.set_auth_context(raisin_models::auth::AuthContext::system())?;
        tx.set_validate_schema(true)?;
        tx.put_node(WS, node).await?;
        Transaction::commit(&tx).await
    }

    async fn read(&self, branch: &str, id: &str) -> Result<Node> {
        Ok(self
            .storage
            .nodes()
            .get(self.scope(branch), id, None)
            .await?
            .unwrap_or_else(|| panic!("node '{id}' must be readable on '{branch}'")))
    }

    /// Resolve a node property's `secret://` reference against a branch's store.
    fn reveal(&self, branch: &str, node: &Node, property: &str) -> Result<Vec<u8>> {
        let reference = reference_of(node, property);
        Ok(self
            .store()
            .get(&self.secrets(branch), &reference.name, reference.version)?)
    }
}

fn node(id: &str, node_type: &str, properties: HashMap<String, PropertyValue>) -> Node {
    Node {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/{id}"),
        node_type: node_type.to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version: 1,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: None,
        created_by: None,
        translations: None,
        tenant_id: None,
        workspace: Some(WS.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

fn props(pairs: &[(&str, &str)]) -> HashMap<String, PropertyValue> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), PropertyValue::String(v.to_string())))
        .collect()
}

fn reference_of(node: &Node, property: &str) -> SecretRef {
    match node.properties.get(property) {
        Some(PropertyValue::String(s)) => SecretRef::parse(s)
            .unwrap_or_else(|e| panic!("'{property}' must hold a secret reference: {e}")),
        other => panic!("expected a string at '{property}', got {other:?}"),
    }
}

// ---- 1. promotion -------------------------------------------------------

/// THE REGRESSION TEST for the live bug.
///
/// The reference is branch-agnostic and the store is branch-scoped, so without
/// the record travelling with the node, the promoted node on `staging` holds a
/// reference to a secret that does not exist there. Nothing reports it: the
/// promotion succeeds and the node reads back looking complete.
///
/// Drives `copy_nodes_across_branches` — the SAME entry point Studio's
/// branch-based publish/merge uses (`raisin-transport-ws/src/handlers/
/// branches.rs::handle_branch_copy_nodes`, which passes these arguments straight
/// through and only emits node events afterwards). Deliberately NOT
/// `publish_tree`: that flips `published_at` through `update_impl` on the same
/// branch, never crosses a branch boundary, and so cannot produce this bug.
#[tokio::test]
async fn a_promoted_node_can_still_reveal_its_secret_on_the_target_branch() -> Result<()> {
    let env = Env::new("secret-promote").await?;
    env.seed_node_type("vault:Connection").await?;
    env.empty_branch("staging").await?;

    env.write(
        MAIN,
        &node(
            "conn-p",
            "vault:Connection",
            props(&[("host", "imap.example.com"), ("password", "hunter2")]),
        ),
    )
    .await?;

    let on_main = env.read(MAIN, "conn-p").await?;
    let promoted_reference = reference_of(&on_main, "password");

    env.storage
        .nodes()
        .copy_nodes_across_branches(
            &env.tenant,
            REPO,
            MAIN,
            "staging",
            WS,
            &["/conn-p".to_string()],
            true,
            false,
            None,
            None,
        )
        .await?;

    let on_staging = env.read("staging", "conn-p").await?;
    assert_eq!(
        reference_of(&on_staging, "password"),
        promoted_reference,
        "a promotion preserves the node id, so the reference must be unchanged"
    );
    assert_eq!(
        env.reveal("staging", &on_staging, "password")?,
        b"hunter2",
        "the promoted node's reference must RESOLVE on the target branch"
    );

    // The source is untouched: a promotion copies, it does not move.
    assert_eq!(env.reveal(MAIN, &on_main, "password")?, b"hunter2");
    Ok(())
}

// ---- 2. copy ------------------------------------------------------------

/// A copy mints a new node id, so it must get its OWN secret. If it kept the
/// source's name the two nodes would share one value — and the sharpest way to
/// show that is to delete the source: a shared secret dies with it.
#[tokio::test]
async fn a_copied_node_owns_its_secret_and_outlives_the_source() -> Result<()> {
    let env = Env::new("secret-copy").await?;
    env.seed_node_type("vault:Connection").await?;

    env.write(
        MAIN,
        &node(
            "conn-src",
            "vault:Connection",
            props(&[("host", "imap.example.com"), ("password", "hunter2")]),
        ),
    )
    .await?;

    let copy = env
        .storage
        .nodes()
        .copy_node(env.scope(MAIN), "/conn-src", "/", Some("conn-copy"), None)
        .await?;

    assert_ne!(copy.id, "conn-src", "a copy mints a new id");

    let copied = env.read(MAIN, &copy.id).await?;
    let reference = reference_of(&copied, "password");
    assert_eq!(
        reference.name,
        format!("node/{}/password", copy.id),
        "the copy's reference must name the COPY, not the source"
    );
    assert_eq!(env.reveal(MAIN, &copied, "password")?, b"hunter2");

    // Delete the SOURCE. A shared secret would die with it.
    assert!(
        env.storage
            .nodes()
            .delete(env.scope(MAIN), "conn-src", DeleteNodeOptions::default())
            .await?
    );

    assert_eq!(
        env.reveal(MAIN, &copied, "password")?,
        b"hunter2",
        "deleting the SOURCE must not destroy the COPY's secret"
    );
    Ok(())
}

// ---- 3. delete ----------------------------------------------------------

/// Deleting a node retires its secrets — but only by APPENDING a tombstone.
/// Older node revisions still carry `secret://name@N`, and a time-travel read of
/// one has to keep resolving, so prior versions must survive.
#[tokio::test]
async fn deleting_a_node_retires_its_secrets_while_pinned_versions_survive() -> Result<()> {
    let env = Env::new("secret-delete").await?;
    env.seed_node_type("vault:Connection").await?;

    env.write(
        MAIN,
        &node(
            "conn-d",
            "vault:Connection",
            props(&[("host", "h"), ("password", "first-value")]),
        ),
    )
    .await?;
    // A second value, so there is a version 1 to pin and a version 2 to retire.
    env.write(
        MAIN,
        &node(
            "conn-d",
            "vault:Connection",
            props(&[("host", "h"), ("password", "second-value")]),
        ),
    )
    .await?;

    let before = env.read(MAIN, "conn-d").await?;
    let name = reference_of(&before, "password").name;
    assert_eq!(env.reveal(MAIN, &before, "password")?, b"second-value");

    assert!(
        env.storage
            .nodes()
            .delete(env.scope(MAIN), "conn-d", DeleteNodeOptions::default())
            .await?
    );

    let store = env.store();
    match store.get(&env.secrets(MAIN), &name, None) {
        Err(SecretError::Gone { .. }) => {}
        other => panic!("the newest version must read as Gone after a delete, got {other:?}"),
    }

    assert_eq!(
        store.get(&env.secrets(MAIN), &name, Some(1))?,
        b"first-value",
        "a pinned older version must still resolve, or time travel breaks"
    );
    Ok(())
}

// ---- 4. field clear -----------------------------------------------------

/// Clearing ONE encrypted field retires that secret and leaves the node's other
/// one alone. Without this the old password stays readable through
/// `secret://node/{id}/password` forever, with nothing on the node to show it
/// still exists.
#[tokio::test]
async fn clearing_an_encrypted_field_retires_only_that_secret() -> Result<()> {
    let env = Env::new("secret-clear").await?;
    env.seed_node_type("vault:Connection").await?;

    env.write(
        MAIN,
        &node(
            "conn-c",
            "vault:Connection",
            props(&[("host", "h"), ("password", "hunter2"), ("token", "tok-abc")]),
        ),
    )
    .await?;

    let before = env.read(MAIN, "conn-c").await?;
    let password = reference_of(&before, "password").name;
    let token = reference_of(&before, "token").name;

    // Same node, `password` gone.
    env.write(
        MAIN,
        &node(
            "conn-c",
            "vault:Connection",
            props(&[("host", "h"), ("token", "tok-abc")]),
        ),
    )
    .await?;

    let store = env.store();
    match store.get(&env.secrets(MAIN), &password, None) {
        Err(SecretError::Gone { .. }) => {}
        other => panic!("the cleared field's secret must read as Gone, got {other:?}"),
    }
    assert_eq!(
        store.get(&env.secrets(MAIN), &token, None)?,
        b"tok-abc",
        "the OTHER encrypted field on the same node must be untouched"
    );
    Ok(())
}

/// Rewriting a field is not clearing it. The new value mints a new version under
/// the SAME name, so a diff that compared paths rather than names would tombstone
/// the value it had just written.
#[tokio::test]
async fn rewriting_an_encrypted_field_does_not_retire_it() -> Result<()> {
    let env = Env::new("secret-rewrite").await?;
    env.seed_node_type("vault:Connection").await?;

    for value in ["first", "second"] {
        env.write(
            MAIN,
            &node(
                "conn-r",
                "vault:Connection",
                props(&[("host", "h"), ("password", value)]),
            ),
        )
        .await?;
    }

    let after = env.read(MAIN, "conn-r").await?;
    assert_eq!(env.reveal(MAIN, &after, "password")?, b"second");
    Ok(())
}

// ---- 5. fork ------------------------------------------------------------

/// A fork copies `cf::SECRETS` wholesale through `BRANCH_CF_REGISTRY`. This has
/// always worked and must keep working — it is the reason the promotion gap went
/// unnoticed, so it is worth an explicit guard.
#[tokio::test]
async fn a_fork_still_carries_secrets() -> Result<()> {
    let env = Env::new("secret-fork").await?;
    env.seed_node_type("vault:Connection").await?;

    env.write(
        MAIN,
        &node(
            "conn-f",
            "vault:Connection",
            props(&[("host", "h"), ("password", "hunter2")]),
        ),
    )
    .await?;

    env.fork(MAIN, "feature").await?;

    let on_fork = env.read("feature", "conn-f").await?;
    assert_eq!(
        env.reveal("feature", &on_fork, "password")?,
        b"hunter2",
        "a fork must inherit the sealed record, not just the reference"
    );
    Ok(())
}
