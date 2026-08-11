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

//! Auto-vaulting on the transaction write path.
//!
//! A plaintext value written into a field declared `encrypted: true` must be
//! sealed into the secret store and replaced by a `secret://` reference BEFORE
//! anything durable sees it. The sharpest assertion in this file is
//! [`plaintext_is_not_findable_through_any_index`]: index entries key on the
//! VALUE, so a plaintext password indexed even once turns
//! `properties->>'password'::String = '<guess>'` into a working oracle, and the
//! entries carry the revision so no later fix removes them.

use std::collections::HashMap;
use std::sync::{Arc, Once};

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_crypto::{Keyring, SecretBox};
use raisin_error::Result;
use raisin_models::nodes::properties::value::Element;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::secret_ref::SecretRef;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::secret_store::{SecretScope, SecretStore};
use raisin_rocksdb::{fractional_index, RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalContext;
use raisin_storage::{
    BranchRepository, CommitMetadata, ElementTypeRepository, NodeRepository, NodeTypeRepository,
    RegistryRepository, RepositoryManagementRepository, Storage, Transaction,
};
use tempfile::TempDir;

const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "content";

/// The node-secret crypto family rejects v1 envelopes on read, so the store
/// refuses to WRITE one. Same `Once` discipline as the store's own unit tests:
/// the variable is only ever set, never cleared, so parallel tests in this
/// binary cannot race on it.
static EMIT_V2: Once = Once::new();

fn enable_v2_emission() {
    EMIT_V2.call_once(|| std::env::set_var("RAISIN_CRYPTO_EMIT_V2", "1"));
}

/// A storage with an explicitly installed secret store, so the test keyring is
/// deterministic and no `RAISIN_MASTER_KEYS` leaks into sibling tests in this
/// binary.
async fn fixture(tenant: &str) -> Result<(TempDir, Arc<RocksDBStorage>)> {
    enable_v2_emission();

    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);

    let keys = Arc::new(Keyring::new(vec![(1, [7u8; 32])], 1).unwrap());
    assert!(
        storage.set_secret_store(Arc::new(SecretStore::new(
            storage.db().clone(),
            Arc::new(SecretBox::with_keyring(keys)),
            "vault-test-node",
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
                default_branch: BRANCH.to_string(),
                description: None,
                tags: HashMap::new(),
            },
        )
        .await?;
    storage
        .branches()
        .create_branch(tenant, REPO, BRANCH, "system", None, None, false, false)
        .await?;

    let workspace_service = WorkspaceService::new(storage.clone());
    let mut workspace = Workspace::new(WS.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    workspace_service.put(tenant, REPO, workspace).await?;

    Ok((temp_dir, storage))
}

fn node_type_from(value: serde_json::Value) -> NodeType {
    serde_json::from_value(value).expect("node type fixture must deserialize")
}

/// `host` is ordinary, `password` is a secret. Two properties so a writer that
/// rewrites everything it walks is visibly different from one that consults the
/// schema.
async fn seed_flat_node_type(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    name: &str,
) -> Result<()> {
    storage
        .node_types()
        .create(
            BranchScope::new(tenant, REPO, BRANCH),
            node_type_from(serde_json::json!({
                "name": name,
                "allowed_children": ["*"],
                "properties": [
                    { "name": "host", "type": "String" },
                    { "name": "password", "type": "String", "encrypted": true },
                ],
            })),
            CommitMetadata::system("seed node type"),
        )
        .await?;
    Ok(())
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

fn props(pairs: &[(&str, PropertyValue)]) -> HashMap<String, PropertyValue> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn text(s: &str) -> PropertyValue {
    PropertyValue::String(s.to_string())
}

/// Write one node through the transaction path and commit.
async fn write(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    node: &Node,
    validate_schema: bool,
) -> Result<()> {
    let tx = storage.begin().await?;
    tx.set_tenant_repo(tenant, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_auth_context(raisin_models::auth::AuthContext::system())?;
    tx.set_validate_schema(validate_schema)?;
    tx.put_node(WS, node).await?;
    Transaction::commit(&tx).await
}

fn stored_property(node: &Node, key: &str) -> String {
    match node.properties.get(key) {
        Some(PropertyValue::String(s)) => s.clone(),
        other => panic!("expected a string at '{key}', got {other:?}"),
    }
}

async fn read_back(storage: &Arc<RocksDBStorage>, tenant: &str, id: &str) -> Result<Node> {
    Ok(storage
        .nodes()
        .get(
            raisin_storage::StorageScope::new(tenant, REPO, BRANCH, WS),
            id,
            None,
        )
        .await?
        .unwrap_or_else(|| panic!("node '{id}' must be readable")))
}

fn scope(tenant: &str) -> SecretScope {
    SecretScope::new(tenant, REPO, BRANCH)
}

// ---- the happy path -----------------------------------------------------

/// Plaintext in, reference out — and the plaintext is retrievable from the
/// store, so nothing was lost in the swap.
#[tokio::test]
async fn plaintext_is_sealed_and_the_property_holds_a_reference() -> Result<()> {
    let tenant = "vault-basic";
    let (_dir, storage) = fixture(tenant).await?;
    seed_flat_node_type(&storage, tenant, "vault:Connection").await?;

    write(
        &storage,
        tenant,
        &node(
            "conn-1",
            "vault:Connection",
            props(&[
                ("host", text("imap.example.com")),
                ("password", text("hunter2")),
            ]),
        ),
        true,
    )
    .await?;

    let stored = read_back(&storage, tenant, "conn-1").await?;

    assert_eq!(
        stored_property(&stored, "host"),
        "imap.example.com",
        "an ordinary property must be untouched"
    );

    let reference = stored_property(&stored, "password");
    let parsed = SecretRef::parse(&reference).expect("the property must hold a secret reference");
    assert_eq!(parsed.name, "node/conn-1/password");
    assert_eq!(
        parsed.version,
        Some(1),
        "the stored reference pins the version it minted"
    );

    let store = storage.secret_store()?;
    assert_eq!(
        store.get(&scope(tenant), &parsed.name, parsed.version)?,
        b"hunter2",
        "the store must return the original plaintext"
    );

    Ok(())
}

/// THE ORACLE TEST.
///
/// Index entries key on the VALUE. If the plaintext reaches
/// `PROPERTY_INDEX` / `UNIQUE_INDEX` / `COMPOUND_INDEX`, an equality scan
/// becomes a guess-checker, permanently — those entries carry the revision, so
/// nothing rewrites them later. The assertion is deliberately wider than the
/// index: the plaintext must not appear in ANY column family except `secrets`,
/// which also covers the node blob and the replication oplog.
#[tokio::test]
async fn plaintext_is_not_findable_through_any_index() -> Result<()> {
    let tenant = "vault-oracle";
    let (_dir, storage) = fixture(tenant).await?;
    seed_flat_node_type(&storage, tenant, "vault:Connection").await?;

    // A value no schema, key or path could contain by accident.
    let secret = "correct-horse-battery-staple-9d1f";
    write(
        &storage,
        tenant,
        &node(
            "conn-oracle",
            "vault:Connection",
            props(&[
                ("host", text("imap.example.com")),
                ("password", text(secret)),
            ]),
        ),
        true,
    )
    .await?;

    // Guard against a vacuous pass: the node must actually exist, and hold a
    // reference. Without this, "the plaintext is nowhere" would also be true of
    // a write that silently did nothing.
    let stored = read_back(&storage, tenant, "conn-oracle").await?;
    assert!(SecretRef::is_secret_ref(&stored_property(
        &stored, "password"
    )));

    let db = storage.db();
    let needle = secret.as_bytes();
    let mut offenders: Vec<String> = Vec::new();

    for cf_name in raisin_rocksdb::all_column_family_names() {
        if cf_name == raisin_rocksdb::cf::SECRETS {
            continue; // the one place ciphertext lives; plaintext never does
        }
        let Some(handle) = db.cf_handle(cf_name) else {
            continue;
        };
        for item in db.iterator_cf(handle, rocksdb::IteratorMode::Start) {
            let (key, value) = item.expect("iteration must succeed");
            if contains(&key, needle) || contains(&value, needle) {
                offenders.push((*cf_name).to_string());
                break;
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "the plaintext secret reached these column families: {offenders:?} — an entry keyed on \
         it makes an equality scan an oracle"
    );

    Ok(())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// ---- versioning ---------------------------------------------------------

/// A read-modify-write round trip must be free. Clients read a reference, so
/// writing one back unchanged has to be a no-op — otherwise every save of an
/// unrelated field mints a secret version.
#[tokio::test]
async fn writing_a_reference_back_does_not_mint_a_version() -> Result<()> {
    let tenant = "vault-roundtrip";
    let (_dir, storage) = fixture(tenant).await?;
    seed_flat_node_type(&storage, tenant, "vault:Connection").await?;

    write(
        &storage,
        tenant,
        &node(
            "conn-rt",
            "vault:Connection",
            props(&[("host", text("a")), ("password", text("s3cret"))]),
        ),
        true,
    )
    .await?;

    let first = read_back(&storage, tenant, "conn-rt").await?;
    let reference = stored_property(&first, "password");

    // Exactly what a client does: read the node, change something else, save.
    write(
        &storage,
        tenant,
        &node(
            "conn-rt",
            "vault:Connection",
            props(&[("host", text("b")), ("password", text(&reference))]),
        ),
        true,
    )
    .await?;

    let second = read_back(&storage, tenant, "conn-rt").await?;
    assert_eq!(stored_property(&second, "host"), "b");
    assert_eq!(
        stored_property(&second, "password"),
        reference,
        "the reference must survive verbatim"
    );

    let versions = storage
        .secret_store()?
        .list_versions(&scope(tenant), "node/conn-rt/password")?;
    assert_eq!(
        versions.len(),
        1,
        "a round trip must not mint a version, got {versions:?}"
    );

    Ok(())
}

/// New plaintext means a human typed a new value, so it always appends — and
/// the old version keeps resolving, which is what makes a pinned reference on
/// an older node revision readable.
#[tokio::test]
async fn new_plaintext_mints_a_second_version_and_the_first_still_resolves() -> Result<()> {
    let tenant = "vault-rotate";
    let (_dir, storage) = fixture(tenant).await?;
    seed_flat_node_type(&storage, tenant, "vault:Connection").await?;

    for value in ["first-pass", "second-pass"] {
        write(
            &storage,
            tenant,
            &node(
                "conn-rot",
                "vault:Connection",
                props(&[("host", text("a")), ("password", text(value))]),
            ),
            true,
        )
        .await?;
    }

    let stored = read_back(&storage, tenant, "conn-rot").await?;
    let parsed = SecretRef::parse(&stored_property(&stored, "password")).unwrap();
    assert_eq!(parsed.version, Some(2), "a new value mints a new version");

    let store = storage.secret_store()?;
    let name = "node/conn-rot/password";
    assert_eq!(store.get(&scope(tenant), name, Some(2))?, b"second-pass");
    assert_eq!(
        store.get(&scope(tenant), name, Some(1))?,
        b"first-pass",
        "@1 must keep resolving for older node revisions"
    );

    Ok(())
}

// ---- nesting ------------------------------------------------------------

/// A secret declared by an ELEMENT type, reached through the property walk, is
/// vaulted under its dot path — the same path format every index uses.
#[tokio::test]
async fn a_nested_element_field_vaults_under_its_dot_path() -> Result<()> {
    let tenant = "vault-nested";
    let (_dir, storage) = fixture(tenant).await?;

    storage
        .element_types()
        .create(
            BranchScope::new(tenant, REPO, BRANCH),
            serde_json::from_value::<ElementType>(serde_json::json!({
                "name": "vault:Cred",
                "fields": [
                    { "$type": "TextField", "name": "label" },
                    { "$type": "TextField", "name": "token", "encrypted": true },
                ],
            }))
            .expect("element type fixture must deserialize"),
            CommitMetadata::system("seed element type"),
        )
        .await?;

    // `hero` is an Element-typed property, which is what makes the gate report
    // `needs_deep_walk` — a NodeType cannot enumerate what an element declares.
    storage
        .node_types()
        .create(
            BranchScope::new(tenant, REPO, BRANCH),
            node_type_from(serde_json::json!({
                "name": "vault:Page",
                "allowed_children": ["*"],
                "properties": [ { "name": "hero", "type": "Element" } ],
            })),
            CommitMetadata::system("seed node type"),
        )
        .await?;

    let hero = PropertyValue::Element(Element {
        uuid: "hero-1".to_string(),
        element_type: "vault:Cred".to_string(),
        content: props(&[("label", text("prod")), ("token", text("tok-abc123"))]),
    });

    write(
        &storage,
        tenant,
        &node("page-1", "vault:Page", props(&[("hero", hero)])),
        false,
    )
    .await?;

    let stored = read_back(&storage, tenant, "page-1").await?;
    let content = match stored.properties.get("hero") {
        Some(PropertyValue::Element(e)) => e.content.clone(),
        other => panic!("expected an element at 'hero', got {other:?}"),
    };
    assert_eq!(
        content.get("label"),
        Some(&text("prod")),
        "a non-secret element field must be untouched"
    );

    let reference = match content.get("token") {
        Some(PropertyValue::String(s)) => s.clone(),
        other => panic!("expected a string at 'hero.token', got {other:?}"),
    };
    let parsed = SecretRef::parse(&reference).expect("the element field must hold a reference");
    assert_eq!(
        parsed.name, "node/page-1/hero.token",
        "the secret name carries the walker's dot path"
    );
    assert_eq!(
        storage
            .secret_store()?
            .get(&scope(tenant), &parsed.name, parsed.version)?,
        b"tok-abc123"
    );

    Ok(())
}

// ---- the fast path ------------------------------------------------------

/// The ~99.9% case: a NodeType with no encrypted field anywhere writes exactly
/// what it was given, and the gate caches `None` so the next write short-circuits
/// before any walk.
#[tokio::test]
async fn a_type_without_encrypted_fields_is_untouched() -> Result<()> {
    let tenant = "vault-plain";
    let (_dir, storage) = fixture(tenant).await?;

    storage
        .node_types()
        .create(
            BranchScope::new(tenant, REPO, BRANCH),
            node_type_from(serde_json::json!({
                "name": "vault:Plain",
                "allowed_children": ["*"],
                "properties": [ { "name": "password", "type": "String" } ],
            })),
            CommitMetadata::system("seed node type"),
        )
        .await?;

    write(
        &storage,
        tenant,
        &node(
            "plain-1",
            "vault:Plain",
            props(&[("password", text("not-declared-secret"))]),
        ),
        true,
    )
    .await?;

    let stored = read_back(&storage, tenant, "plain-1").await?;
    assert_eq!(
        stored_property(&stored, "password"),
        "not-declared-secret",
        "a property that is merely NAMED password is not a secret"
    );

    // The gate answered, and answered `None` — so the next write to this type
    // costs one map lookup and skips the walk entirely.
    let gate = raisin_core::services::encrypted_fields::global().get(
        &raisin_core::services::encrypted_fields::EncryptedFieldsCache::key(
            tenant,
            REPO,
            BRANCH,
            "vault:Plain",
        ),
    );
    assert_eq!(
        gate,
        raisin_core::services::encrypted_fields::EncryptedFields::None,
        "the fast path depends on this being cached as None, not Unknown"
    );

    Ok(())
}

// ---- the per-transaction memo -------------------------------------------

/// Bulk SQL DML can `put_node` the same node twice in ONE transaction, and the
/// transaction's HLC is allocated once — so both writes land on one node
/// revision, the second overwriting the first. A byte-identical rewrite must
/// therefore reuse the version it already minted, or `@1` becomes an orphan and
/// the version counter tracks statement count rather than value changes.
#[tokio::test]
async fn two_writes_of_one_node_in_one_transaction_mint_one_version() -> Result<()> {
    let tenant = "vault-memo";
    let (_dir, storage) = fixture(tenant).await?;
    seed_flat_node_type(&storage, tenant, "vault:Connection").await?;

    let tx = storage.begin().await?;
    tx.set_tenant_repo(tenant, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_auth_context(raisin_models::auth::AuthContext::system())?;
    let node = node(
        "conn-memo",
        "vault:Connection",
        props(&[("host", text("a")), ("password", text("same-value"))]),
    );
    tx.put_node(WS, &node).await?;
    tx.put_node(WS, &node).await?;
    Transaction::commit(&tx).await?;

    let stored = read_back(&storage, tenant, "conn-memo").await?;
    let parsed = SecretRef::parse(&stored_property(&stored, "password")).unwrap();
    assert_eq!(parsed.version, Some(1));
    assert_eq!(
        storage
            .secret_store()?
            .list_versions(&scope(tenant), "node/conn-memo/password")?
            .len(),
        1,
        "an identical rewrite in one transaction must not append"
    );

    Ok(())
}

/// The memo is keyed by plaintext hash, not by name alone. If the second
/// statement carries a DIFFERENT value, reusing the first version would store
/// the wrong secret under the reference that survives the batch.
#[tokio::test]
async fn a_changed_value_in_one_transaction_still_mints_a_second_version() -> Result<()> {
    let tenant = "vault-memo-change";
    let (_dir, storage) = fixture(tenant).await?;
    seed_flat_node_type(&storage, tenant, "vault:Connection").await?;

    let tx = storage.begin().await?;
    tx.set_tenant_repo(tenant, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_auth_context(raisin_models::auth::AuthContext::system())?;
    for value in ["first", "second"] {
        tx.put_node(
            WS,
            &node(
                "conn-memo2",
                "vault:Connection",
                props(&[("host", text("a")), ("password", text(value))]),
            ),
        )
        .await?;
    }
    Transaction::commit(&tx).await?;

    let stored = read_back(&storage, tenant, "conn-memo2").await?;
    let parsed = SecretRef::parse(&stored_property(&stored, "password")).unwrap();
    assert_eq!(
        parsed.version,
        Some(2),
        "the surviving reference must point at the surviving value"
    );
    assert_eq!(
        storage
            .secret_store()?
            .get(&scope(tenant), &parsed.name, parsed.version)?,
        b"second"
    );

    Ok(())
}

// ---- the repository write path ------------------------------------------

/// `storage.nodes().create(...)` does NOT go through a transaction — it is the
/// direct repository path (`add_impl`), one of the four low-level write
/// functions. It must vault too, or every caller that uses it writes plaintext
/// while the transaction path looks correct.
#[tokio::test]
async fn the_repository_create_path_vaults_too() -> Result<()> {
    let tenant = "vault-repo-create";
    let (_dir, storage) = fixture(tenant).await?;
    seed_flat_node_type(&storage, tenant, "vault:Connection").await?;

    storage
        .nodes()
        .create(
            raisin_storage::StorageScope::new(tenant, REPO, BRANCH, WS),
            node(
                "repo-conn",
                "vault:Connection",
                props(&[("host", text("a")), ("password", text("repo-plaintext"))]),
            ),
            raisin_storage::CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                operation_meta: None,
            },
        )
        .await?;

    let stored = read_back(&storage, tenant, "repo-conn").await?;
    let parsed = SecretRef::parse(&stored_property(&stored, "password"))
        .expect("the repository path must store a reference, not the plaintext");
    assert_eq!(parsed.name, "node/repo-conn/password");
    assert_eq!(
        storage
            .secret_store()?
            .get(&scope(tenant), &parsed.name, parsed.version)?,
        b"repo-plaintext"
    );

    Ok(())
}

/// The repository UPDATE path (`update_impl`) — which is also what the
/// single-property write `update_property_by_path_impl` funnels into, so this
/// covers `queries/property.rs` as well.
#[tokio::test]
async fn the_repository_update_path_vaults_too() -> Result<()> {
    let tenant = "vault-repo-update";
    let (_dir, storage) = fixture(tenant).await?;
    seed_flat_node_type(&storage, tenant, "vault:Connection").await?;

    let scope_ = raisin_storage::StorageScope::new(tenant, REPO, BRANCH, WS);
    storage
        .nodes()
        .create(
            scope_,
            node(
                "repo-upd",
                "vault:Connection",
                props(&[("host", text("a")), ("password", text("first"))]),
            ),
            raisin_storage::CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                operation_meta: None,
            },
        )
        .await?;

    storage
        .nodes()
        .update(
            raisin_storage::StorageScope::new(tenant, REPO, BRANCH, WS),
            node(
                "repo-upd",
                "vault:Connection",
                props(&[("host", text("b")), ("password", text("second"))]),
            ),
            raisin_storage::UpdateNodeOptions::default(),
        )
        .await?;

    let stored = read_back(&storage, tenant, "repo-upd").await?;
    let parsed = SecretRef::parse(&stored_property(&stored, "password")).unwrap();
    assert_eq!(
        parsed.version,
        Some(2),
        "a repository update with new plaintext appends a version"
    );

    let store = storage.secret_store()?;
    let name = "node/repo-upd/password";
    assert_eq!(store.get(&scope(tenant), name, Some(2))?, b"second");
    assert_eq!(store.get(&scope(tenant), name, Some(1))?, b"first");

    Ok(())
}

/// An INHERITED `encrypted` declaration must be honoured. This is what the
/// shared-resolver refactor buys: the repository path drives the same
/// inheritance walk as the transaction path, so a secret declared on a parent
/// type is not silently written in plaintext by the layer that "only" does a
/// flat NodeType lookup for its other validations.
#[tokio::test]
async fn an_inherited_encrypted_declaration_is_honoured_on_the_repository_path() -> Result<()> {
    let tenant = "vault-inherited";
    let (_dir, storage) = fixture(tenant).await?;

    storage
        .node_types()
        .create(
            BranchScope::new(tenant, REPO, BRANCH),
            node_type_from(serde_json::json!({
                "name": "vault:BaseCredential",
                "allowed_children": ["*"],
                "properties": [ { "name": "password", "type": "String", "encrypted": true } ],
            })),
            CommitMetadata::system("seed base"),
        )
        .await?;
    storage
        .node_types()
        .create(
            BranchScope::new(tenant, REPO, BRANCH),
            node_type_from(serde_json::json!({
                "name": "vault:DerivedCredential",
                "extends": "vault:BaseCredential",
                "allowed_children": ["*"],
                "properties": [ { "name": "host", "type": "String" } ],
            })),
            CommitMetadata::system("seed derived"),
        )
        .await?;

    storage
        .nodes()
        .create(
            raisin_storage::StorageScope::new(tenant, REPO, BRANCH, WS),
            node(
                "inherited-1",
                "vault:DerivedCredential",
                props(&[("host", text("h")), ("password", text("inherited-secret"))]),
            ),
            raisin_storage::CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                operation_meta: None,
            },
        )
        .await?;

    let stored = read_back(&storage, tenant, "inherited-1").await?;
    let parsed = SecretRef::parse(&stored_property(&stored, "password"))
        .expect("a secret declared on the PARENT type must still be vaulted");
    assert_eq!(
        storage
            .secret_store()?
            .get(&scope(tenant), &parsed.name, parsed.version)?,
        b"inherited-secret"
    );

    Ok(())
}

// ---- determinate absence vs. indeterminate failure ----------------------

/// A NodeType that does not exist is a DETERMINATE answer, not a failure: no
/// schema exists, so no schema declares `encrypted: true`, so there is nothing
/// to vault. The write must SUCCEED and store the string as it was given.
///
/// Getting this wrong makes every node of an unregistered type unwritable —
/// which is a much bigger blast radius than the leak it would be guarding
/// against, and there is no leak here to guard against.
#[tokio::test]
async fn an_unregistered_node_type_writes_its_string_as_plaintext() -> Result<()> {
    let tenant = "vault-unregistered";
    let (_dir, storage) = fixture(tenant).await?;

    write(
        &storage,
        tenant,
        &node(
            "ghost-ok",
            "vault:NeverDeclared",
            props(&[("password", text("no-schema-says-otherwise"))]),
        ),
        false,
    )
    .await?;

    let stored = read_back(&storage, tenant, "ghost-ok").await?;
    assert_eq!(
        stored_property(&stored, "password"),
        "no-schema-says-otherwise",
        "with no schema there is nothing declaring this a secret"
    );

    // And nothing was vaulted on the way through.
    assert!(
        storage
            .secret_store()?
            .list_versions(&scope(tenant), "node/ghost-ok/password")?
            .is_empty(),
        "an absent schema must not mint a secret"
    );

    Ok(())
}

/// An INDETERMINATE resolver failure — here a circular `extends`, which the
/// resolver reports as a validation error rather than a not-found — means the
/// type may exist AND may declare secrets, and we could not read it. That must
/// still refuse the write.
///
/// This is the case the fail-closed rule is actually for, and the reason
/// `NotFound` had to be separated out rather than the whole branch relaxed.
#[tokio::test]
async fn an_indeterminate_resolver_failure_still_refuses() -> Result<()> {
    let tenant = "vault-indeterminate";
    let (_dir, storage) = fixture(tenant).await?;

    // A extends B, B extends A. Resolution reports a circular dependency, so
    // whether either declares an encrypted field is unknowable.
    for (name, extends) in [("vault:Loop", "vault:Loop2"), ("vault:Loop2", "vault:Loop")] {
        storage
            .node_types()
            .create(
                BranchScope::new(tenant, REPO, BRANCH),
                node_type_from(serde_json::json!({
                    "name": name,
                    "extends": extends,
                    "allowed_children": ["*"],
                })),
                CommitMetadata::system("seed circular type"),
            )
            .await?;
    }

    let result = write(
        &storage,
        tenant,
        &node(
            "loop-1",
            "vault:Loop",
            props(&[("password", text("would-have-been-plaintext"))]),
        ),
        false,
    )
    .await;

    let err = result.expect_err("an unreadable schema plus strings must refuse the write");
    let message = err.to_string();
    assert!(
        message.contains("vault:Loop") && message.contains("encrypted"),
        "the refusal must name the type and the reason, got: {message}"
    );

    assert!(
        storage
            .nodes()
            .get(
                raisin_storage::StorageScope::new(tenant, REPO, BRANCH, WS),
                "loop-1",
                None,
            )
            .await?
            .is_none(),
        "the refused node must not exist"
    );

    Ok(())
}
