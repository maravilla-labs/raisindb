//! Plan Phase 13a: UNIQUE_INDEX asserted DIRECTLY where a promotion or a
//! replicated write ends a claim. A claim key is `(type, property, value,
//! revision)` with the owner in the value — no node id — so a node giving a
//! value up and a node taking it at one revision write ONE key, and a claim
//! ended in the wrong order (or not at all) is invisible until someone is
//! refused a value nobody holds, or two nodes hold one.
//!
//! Every case is checked on the origin and on replicas fed the oplog as
//! stored, decomposed, and decomposed with deletes moved last.

use crate::cross_branch_displacement_test::{child, delete_on_main, every_peer, replicas};
use crate::cross_branch_prune_replication_test::{
    create, folder, promote_with, with_publish_branch, PUBLISH,
};
use crate::translation_replication_test::{node, receive, Node as Peer};
use crate::translation_substrate_test::{B, R, T, WS};
use raisin_models::nodes::properties::schema::{PropertyType, PropertyValueSchema};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::{Node, NodeType};
use raisin_rocksdb::{cf, keys, OpLogRepository};
use raisin_storage::{
    BranchScope, CommitMetadata, NodeRepository, NodeTypeRepository, Storage, StorageScope,
    UpdateNodeOptions,
};

pub(crate) const ACCOUNT: &str = "test:Account";
pub(crate) const EMAIL: &str = "email";

/// A NodeType with one `unique: true` property, `email`.
pub(crate) fn account_type() -> NodeType {
    let email = PropertyValueSchema {
        name: Some(EMAIL.to_string()),
        property_type: PropertyType::String,
        required: None,
        unique: Some(true),
        default: None,
        constraints: None,
        structure: None,
        items: None,
        value: None,
        meta: None,
        is_translatable: None,
        allow_additional_properties: None,
        index: None,
        spatial: None,
        encrypted: None,
    };
    NodeType {
        id: Some(ACCOUNT.to_string()),
        name: ACCOUNT.to_string(),
        strict: Some(false),
        allowed_children: vec!["*".to_string()],
        indexable: Some(true),
        created_at: Some(chrono::Utc::now()),
        properties: Some(vec![email]),
        compound_indexes: None,
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        immutable: None,
        publishable: Some(true),
        auditable: Some(false),
        index_types: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        is_mixin: None,
    }
}

/// Declare [`account_type`] on main.
pub(crate) async fn with_account_type(peer: &Peer) {
    peer.storage
        .node_types()
        .upsert(
            BranchScope::new(T, R, B),
            account_type(),
            CommitMetadata::system("account type"),
        )
        .await
        .expect("account type");
}

/// An account `name` under `parent` holding `email`.
pub(crate) fn account(name: &str, parent: &Node, email: &str) -> Node {
    let mut n = child(name, parent);
    n.node_type = ACCOUNT.to_string();
    n.properties
        .insert(EMAIL.into(), PropertyValue::String(email.into()));
    n
}

/// Change `node`'s email on main.
pub(crate) async fn set_email(peer: &Peer, node: &mut Node, email: &str) {
    node.properties
        .insert(EMAIL.into(), PropertyValue::String(email.into()));
    let options = UpdateNodeOptions {
        validate_schema: false,
        ..Default::default()
    };
    peer.storage
        .nodes()
        .update(StorageScope::new(T, R, B, WS), node.clone(), options)
        .await
        .expect("update");
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
}

/// The node the newest UNIQUE_INDEX entry for `email` names on `branch`,
/// read RAW (`None`: no entry, or a tombstone).
pub(crate) fn claim_owner(peer: &Peer, branch: &str, email: &str) -> Option<String> {
    let db = peer.storage.db();
    let prefix = keys::unique_index_value_prefix(T, R, branch, WS, ACCOUNT, EMAIL, email);
    let cf = db.cf_handle(cf::UNIQUE_INDEX).unwrap();
    let (key, value) = db.prefix_iterator_cf(cf, &prefix).next()?.unwrap();
    if !key.starts_with(&prefix) {
        return None;
    }
    (!keys::is_tombstone_value(&value)).then(|| String::from_utf8(value.to_vec()).unwrap())
}

/// On every peer's publish branch, each `(email, owner)` holds.
async fn assert_claims(origin: &Peer, expected: &[(&str, Option<&Node>)]) {
    let replicas = replicas(origin).await;
    for (name, peer) in every_peer(origin, &replicas).await {
        for (email, owner) in expected {
            assert_eq!(
                claim_owner(peer, PUBLISH, email),
                owner.map(|n| n.id.clone()),
                "{name}: the claim on {email}"
            );
        }
    }
}

async fn seeded() -> (Peer, Node) {
    let origin = node("origin").await;
    with_publish_branch(&origin).await;
    with_account_type(&origin).await;
    let section = folder("section", "/section");
    create(&origin, &section).await;
    (origin, section)
}

#[tokio::test]
async fn a_value_handed_over_in_one_promotion_names_its_new_owner_in_both_orders() {
    for giver_first in [true, false] {
        let (origin, section) = seeded().await;
        let names = if giver_first { ["a", "b"] } else { ["b", "a"] };
        let mut first = account(names[0], &section, "first@x");
        create(&origin, &first).await;
        let mut second = account(names[1], &section, "second@x");
        create(&origin, &second).await;
        promote_with(&origin, true).await;
        let (giver, taker) = if giver_first {
            (&mut first, &mut second)
        } else {
            (&mut second, &mut first)
        };
        let given = giver.properties[EMAIL].clone();
        let PropertyValue::String(given) = given else {
            unreachable!()
        };
        // The giver gives its value up, then the taker takes it; one promotion.
        let taken = taker.properties[EMAIL].clone();
        let PropertyValue::String(taken) = taken else {
            unreachable!()
        };
        set_email(&origin, giver, "new@x").await;
        set_email(&origin, taker, &given).await;
        promote_with(&origin, true).await;
        assert_claims(
            &origin,
            &[
                (&given, Some(&*taker)),
                ("new@x", Some(&*giver)),
                (&taken, None),
            ],
        )
        .await;
    }
}

#[tokio::test]
async fn a_displaced_nodes_claim_names_its_replacement() {
    let (origin, section) = seeded().await;
    let old = account("page", &section, "v@x");
    create(&origin, &old).await;
    promote_with(&origin, false).await;
    delete_on_main(&origin, &old.id).await;
    let new = account("page", &section, "v@x");
    create(&origin, &new).await;
    promote_with(&origin, false).await;
    assert_claims(&origin, &[("v@x", Some(&new))]).await;
}

#[tokio::test]
async fn a_pruned_nodes_claim_names_its_replacement() {
    let (origin, section) = seeded().await;
    let old = account("old", &section, "v@x");
    create(&origin, &old).await;
    promote_with(&origin, true).await;
    delete_on_main(&origin, &old.id).await;
    let new = account("new", &section, "v@x");
    create(&origin, &new).await;
    promote_with(&origin, true).await;
    assert_claims(&origin, &[("v@x", Some(&new))]).await;
}

/// The origin's oplog after `after_seq`, as stored and decomposed.
fn deliveries(origin: &Peer, after_seq: u64) -> [Vec<raisin_replication::Operation>; 2] {
    let mut ops = OpLogRepository::new(origin.storage.db().clone())
        .get_operations_from_node(T, R, &origin.id)
        .unwrap();
    ops.sort_by_key(|op| op.op_seq);
    ops.retain(|op| op.op_seq > after_seq);
    let decomposed = ops
        .iter()
        .cloned()
        .flat_map(raisin_replication::decompose_operation)
        .collect();
    [ops, decomposed]
}

fn highest_seq(origin: &Peer) -> u64 {
    crate::translation_replication_test::highest_seq(origin)
}

/// Two replicas (as stored, decomposed) holding `account`'s claim on main,
/// with their definitions cache dropped when `cold`.
async fn replicas_with_claim(origin: &Peer, account: &Node, cold: bool) -> Vec<Peer> {
    let mut out = Vec::new();
    for (i, ops) in deliveries(origin, 0).into_iter().enumerate() {
        let replica = node(&format!("replica-{i}")).await;
        receive(&replica, &ops).await;
        let email = match &account.properties[EMAIL] {
            PropertyValue::String(s) => s.clone(),
            _ => unreachable!(),
        };
        assert_eq!(
            claim_owner(&replica, B, &email),
            Some(account.id.clone()),
            "replica-{i}: a warm replicated create claims its value"
        );
        if cold {
            raisin_rocksdb::indexing::compound::defs::invalidate_database(replica.storage.db());
        }
        out.push(replica);
    }
    out
}

#[tokio::test]
async fn a_replicated_delete_ends_the_claim_warm_or_cold() {
    for cold in [false, true] {
        let origin = node("origin").await;
        with_account_type(&origin).await;
        let section = folder("section", "/section");
        create(&origin, &section).await;
        let acct = account("acct", &section, "v@x");
        create(&origin, &acct).await;
        let replicas = replicas_with_claim(&origin, &acct, cold).await;
        let seq = highest_seq(&origin);
        delete_on_main(&origin, &acct.id).await;
        for (i, (replica, ops)) in replicas.iter().zip(deliveries(&origin, seq)).enumerate() {
            receive(replica, &ops).await;
            assert_eq!(
                claim_owner(replica, B, "v@x"),
                None,
                "replica-{i} (cold: {cold}): a deleted node's claim is still live"
            );
        }
    }
}

#[tokio::test]
async fn a_cold_replicated_update_moves_the_claim() {
    let origin = node("origin").await;
    with_account_type(&origin).await;
    let section = folder("section", "/section");
    create(&origin, &section).await;
    let mut acct = account("acct", &section, "v@x");
    create(&origin, &acct).await;
    let replicas = replicas_with_claim(&origin, &acct, true).await;
    let seq = highest_seq(&origin);
    set_email(&origin, &mut acct, "w@x").await;
    for (i, (replica, ops)) in replicas.iter().zip(deliveries(&origin, seq)).enumerate() {
        receive(replica, &ops).await;
        assert_eq!(
            claim_owner(replica, B, "v@x"),
            None,
            "replica-{i}: the value a cold update gave up is still claimed"
        );
        assert_eq!(
            claim_owner(replica, B, "w@x"),
            Some(acct.id.clone()),
            "replica-{i}: a cold update claims its new value"
        );
    }
}
