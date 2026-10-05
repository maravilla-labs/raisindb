//! Plan Phase 13a review: two peers that end up holding the same ops must end
//! up holding the same UNIQUE claims, whatever order the ops reached them in.
//!
//! A claim-ending tombstone used to be skipped whenever the newest claim BELOW
//! it named another node. Whether that other node's claim had arrived yet
//! depends on delivery order, so a peer that wrote the tombstone locally (the
//! concurrent claim not yet received) and a peer that applied it after the
//! concurrent claim disagreed forever: the value free on one, held on the
//! other, a third node admitted on one and refused on the other. The same
//! rule lived in three copies — the delta's ends, the delete tombstoner, and
//! none at all in the transaction delete. Now the only skip is a same-revision
//! key collision (`unique_guard::held_by_other_at`), through one body
//! (`unique_delta::end_claim`).
//!
//! The replicated delete also probed only the version the deleting origin
//! carried: when a concurrent update had moved this peer's version on, the
//! new value's claim outlived the node here while the origin ended it.

use crate::cross_branch_displacement_test::delete_on_main;
use crate::cross_branch_prune_replication_test::folder;
use crate::promotion_unique_test::{set_email, with_account_type, ACCOUNT, EMAIL};
use crate::translation_replication_test::{highest_seq, node, receive, Node as Peer};
use crate::translation_substrate_test::{B, R, T, WS};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::Operation;
use raisin_rocksdb::{cf, keys, OpLogRepository};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};

/// An account at the workspace root holding `email`.
fn root_account(name: &str, email: &str) -> Node {
    let mut n = folder(name, &format!("/{name}"));
    n.node_type = ACCOUNT.to_string();
    n.properties
        .insert(EMAIL.into(), PropertyValue::String(email.into()));
    n
}

async fn create_on(peer: &Peer, n: &Node) {
    let options = CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    };
    peer.storage
        .nodes()
        .create(StorageScope::new(T, R, B, WS), n.clone(), options)
        .await
        .expect("create");
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
}

/// Delete through the TRANSACTION context (SQL DML, WS delete).
async fn tx_delete(peer: &Peer, id: &str) {
    let ctx = peer.storage.begin_context().await.unwrap();
    ctx.set_tenant_repo(T, R).unwrap();
    ctx.set_branch(B).unwrap();
    ctx.set_actor("test-user").unwrap();
    ctx.set_auth_context(AuthContext::system()).unwrap();
    ctx.set_message("delete").unwrap();
    ctx.delete_node(WS, id).await.unwrap();
    ctx.commit().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
}

/// The ops `peer` captured after `after_seq`, in order.
fn ops_after(peer: &Peer, after_seq: u64) -> Vec<Operation> {
    let mut ops = OpLogRepository::new(peer.storage.db().clone())
        .get_operations_from_node(T, R, &peer.id)
        .unwrap();
    ops.sort_by_key(|op| op.op_seq);
    ops.retain(|op| op.op_seq > after_seq);
    ops
}

/// Every raw UNIQUE_INDEX entry of `(ACCOUNT, email, value)` on main, newest
/// first: (revision bytes, owner or "T").
fn entries(peer: &Peer, value: &str) -> Vec<(Vec<u8>, String)> {
    let db = peer.storage.db();
    let prefix = keys::unique_index_value_prefix(T, R, B, WS, ACCOUNT, EMAIL, value);
    let cf = db.cf_handle(cf::UNIQUE_INDEX).unwrap();
    db.prefix_iterator_cf(cf, &prefix)
        .map(Result::unwrap)
        .take_while(|(key, _)| key.starts_with(&prefix))
        .map(|(key, value)| {
            let owner = if keys::is_tombstone_value(&value) {
                "T".to_string()
            } else {
                String::from_utf8(value.to_vec()).unwrap()
            };
            (key[prefix.len()..].to_vec(), owner)
        })
        .collect()
}

/// The owner the newest entry names (`None`: none, or a tombstone).
fn owner(peer: &Peer, value: &str) -> Option<String> {
    entries(peer, value)
        .into_iter()
        .next()
        .filter(|(_, owner)| owner != "T")
        .map(|(_, owner)| owner)
}

async fn peer(id: &str) -> (Peer, u64) {
    let peer = node(id).await;
    with_account_type(&peer).await;
    let seq = highest_seq(&peer);
    (peer, seq)
}

#[derive(Clone, Copy, Debug)]
enum GiveUp {
    Update,
    RepositoryDelete,
    TransactionDelete,
}

#[tokio::test]
async fn two_origins_converge_on_a_claim_whatever_order_the_ops_arrive() {
    for give_up in [
        GiveUp::Update,
        GiveUp::RepositoryDelete,
        GiveUp::TransactionDelete,
    ] {
        let ((p1, s1), (p2, s2)) = (peer("p1").await, peer("p2").await);
        // p1 claims x; p2, not having seen it, claims x too (a concurrent
        // duplicate); p1, not having seen p2's, then gives x up.
        let mut a = root_account("a", "x@x");
        create_on(&p1, &a).await;
        let b = root_account("b", "x@x");
        create_on(&p2, &b).await;
        match give_up {
            GiveUp::Update => set_email(&p1, &mut a, "y@x").await,
            GiveUp::RepositoryDelete => delete_on_main(&p1, &a.id).await,
            GiveUp::TransactionDelete => tx_delete(&p1, &a.id).await,
        }
        // p1 receives p2's claim after its own give-up; p2 receives p1's
        // claim and give-up after its own claim.
        receive(&p1, &ops_after(&p2, s2)).await;
        receive(&p2, &ops_after(&p1, s1)).await;
        for value in ["x@x", "y@x"] {
            assert_eq!(
                entries(&p1, value),
                entries(&p2, value),
                "{give_up:?}: the peers hold different UNIQUE entries for {value}"
            );
        }
    }
}

#[tokio::test]
async fn a_replicated_delete_ends_the_claim_of_a_concurrent_update_it_never_saw() {
    for cold in [false, true] {
        let ((p1, s1), (p2, s2)) = (peer("p1").await, peer("p2").await);
        let a = root_account("a", "x@x");
        create_on(&p1, &a).await;
        let created = highest_seq(&p1);
        receive(&p2, &ops_after(&p1, s1)).await;
        // p2 moves A's value on; p1, not having seen it, deletes A carrying
        // the version with the OLD value.
        let mut on_p2 = a.clone();
        set_email(&p2, &mut on_p2, "y@x").await;
        delete_on_main(&p1, &a.id).await;
        if cold {
            raisin_rocksdb::indexing::compound::defs::invalidate_database(p2.storage.db());
        }
        receive(&p2, &ops_after(&p1, created)).await;
        receive(&p1, &ops_after(&p2, s2)).await;
        for (name, peer) in [("p1", &p1), ("p2", &p2)] {
            for value in ["x@x", "y@x"] {
                assert_eq!(
                    owner(peer, value),
                    None,
                    "{name} (cold: {cold}): a deleted node still claims {value}"
                );
            }
        }
    }
}
