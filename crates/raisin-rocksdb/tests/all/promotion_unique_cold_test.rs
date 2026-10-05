//! Plan Phase 13a review: a replica applying a promotion with its
//! definitions cache COLD finds the claims each node's replaced version
//! holds from the index (`owned_unique_names`). That probe used to ask for
//! the NEWEST entry at or before the promotion's revision R — and a replica
//! applies one node per batch, so once the taker of a value had been applied
//! at R, the giver's probe found the taker's entry, concluded the property
//! was never the giver's claim, and claimed nothing for the giver's new
//! value: a value held by a node with no claim, free for a duplicate.
//!
//! Every case runs in both staging orders and is checked on the origin and
//! on replicas (as stored, decomposed, deletes last) whose definitions cache
//! is dropped just before the promotion arrives.

use crate::cross_branch_displacement_test::{every_peer, replicas_with};
use crate::cross_branch_prune_replication_test::{
    create, folder, promote_with, with_publish_branch,
};
use crate::promotion_unique_test::{account, claim_owner, set_email, with_account_type, EMAIL};
use crate::translation_replication_test::{highest_seq, node, Node as Peer};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

const PUBLISH: &str = crate::cross_branch_prune_replication_test::PUBLISH;

fn email(node: &Node) -> String {
    match &node.properties[EMAIL] {
        PropertyValue::String(s) => s.clone(),
        _ => unreachable!(),
    }
}

/// Two accounts promoted once, created (and so listed and staged) in
/// `names` order.
async fn seeded(names: [&str; 2], emails: [&str; 2]) -> (Peer, Node, Node) {
    let origin = node("origin").await;
    with_publish_branch(&origin).await;
    with_account_type(&origin).await;
    let section = folder("section", "/section");
    create(&origin, &section).await;
    let first = account(names[0], &section, emails[0]);
    create(&origin, &first).await;
    let second = account(names[1], &section, emails[1]);
    create(&origin, &second).await;
    promote_with(&origin, true).await;
    (origin, first, second)
}

/// On the origin and on replicas cold for everything after `split`, each
/// `(email, owner)` holds on publish.
async fn assert_cold_claims(origin: &Peer, split: u64, expected: &[(&str, Option<&Node>)]) {
    let replicas = replicas_with(origin, split, |replica| {
        raisin_rocksdb::indexing::compound::defs::invalidate_database(replica.storage.db());
    })
    .await;
    for (name, peer) in every_peer(origin, &replicas).await {
        for (value, owner) in expected {
            assert_eq!(
                claim_owner(peer, PUBLISH, value),
                owner.map(|n| n.id.clone()),
                "{name}: the claim on {value}"
            );
        }
    }
}

#[tokio::test]
async fn a_cold_replica_claims_both_values_of_a_swap() {
    // Each node both gives a value up and takes the other's, so whichever is
    // applied second probes a value the first has just claimed at R.
    let (origin, mut a, mut b) = seeded(["a", "b"], ["v@x", "w@x"]).await;
    let split = highest_seq(&origin);
    // Swapped on main through a third value (one claim per value there).
    set_email(&origin, &mut a, "tmp@x").await;
    set_email(&origin, &mut b, "v@x").await;
    set_email(&origin, &mut a, "w@x").await;
    promote_with(&origin, true).await;
    assert_cold_claims(
        &origin,
        split,
        &[("v@x", Some(&b)), ("w@x", Some(&a)), ("tmp@x", None)],
    )
    .await;
}

#[tokio::test]
async fn a_cold_replica_claims_the_givers_new_value_of_a_handover_in_both_orders() {
    for giver_first in [true, false] {
        // Accounts are listed (and staged) in creation order: the giver is
        // created first or second.
        let emails = if giver_first {
            ["given@x", "taken@x"]
        } else {
            ["taken@x", "given@x"]
        };
        let (origin, first, second) = seeded(["a", "b"], emails).await;
        let (mut giver, mut taker) = if giver_first {
            (first, second)
        } else {
            (second, first)
        };
        assert_eq!(email(&giver), "given@x");
        let split = highest_seq(&origin);
        set_email(&origin, &mut giver, "new@x").await;
        set_email(&origin, &mut taker, "given@x").await;
        promote_with(&origin, true).await;
        assert_cold_claims(
            &origin,
            split,
            &[
                ("given@x", Some(&taker)),
                ("new@x", Some(&giver)),
                ("taken@x", None),
            ],
        )
        .await;
    }
}
