//! The replication apply path writes PROPERTY_INDEX through the one writer:
//! full puts (until the oracle proves the delta there), membership included.

use super::env::{node, Env, WS};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::{
    Node, INDEXED_MIXIN_KEY, INDEXED_SUPERTYPE_KEY, RESERVED_MIXINS_KEY, RESERVED_SUPERTYPES_KEY,
};
use raisin_replication::{
    operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind},
    OpType, Operation, VectorClock,
};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_storage::Storage;
use std::sync::Arc;

pub(super) fn applicator(env: &Env) -> OperationApplicator {
    OperationApplicator::new(
        env.storage.db().clone(),
        env.storage.event_bus(),
        Arc::new(env.storage.branches_impl().clone()),
    )
}

fn later(offset_ms: u64) -> HLC {
    HLC::new(chrono::Utc::now().timestamp_millis() as u64 + offset_ms, 0)
}

/// Apply a replicated upsert of `node` as an up-to-date origin sends it: its
/// write layer stamped `created_at` / `updated_at` (a fixed instant when the
/// fixture left them unset). [`upsert_as_stored`] sends it verbatim.
pub(super) async fn upsert(
    applicator: &OperationApplicator,
    node: &Node,
    label: &str,
    revision: HLC,
) {
    let mut node = node.clone();
    let stamped = chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("instant");
    node.created_at.get_or_insert(stamped);
    node.updated_at.get_or_insert(stamped);
    upsert_as_stored(applicator, &node, label, revision).await
}

/// [`upsert`] without the origin's stamps: a LEGACY version, written before
/// the write layer stamped timestamps, replicated as its origin stored it.
pub(super) async fn upsert_as_stored(
    applicator: &OperationApplicator,
    node: &Node,
    label: &str,
    revision: HLC,
) {
    let mut node = node.clone();
    node.workspace = Some(WS.to_string());
    let change = ReplicatedNodeChange {
        node,
        parent_id: Some("/".to_string()),
        kind: ReplicatedNodeChangeKind::Upsert,
        cf_order_key: label.to_string(),
    };
    let op = Operation {
        op_id: uuid::Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "peer".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: super::env::TENANT.to_string(),
        repo_id: super::env::REPO.to_string(),
        branch: "main".to_string(),
        op_type: OpType::ApplyRevision {
            branch_head: revision,
            node_changes: vec![change],
        },
        revision: Some(revision),
        actor: "system".to_string(),
        message: None,
        is_system: true,
        agent: None,
        acknowledged_by: Default::default(),
    };
    applicator.apply_operation(&op).await.expect("apply upsert");
}

fn strings(values: &[&str]) -> PropertyValue {
    PropertyValue::Array(
        values
            .iter()
            .map(|v| PropertyValue::String(v.to_string()))
            .collect(),
    )
}

#[tokio::test]
async fn is_a_and_has_mixin_on_replica() -> Result<()> {
    for skip in [false, true] {
        let env = Env::new(skip).await?;
        let replica = applicator(&env);
        let mut page = node("p", "/p", &[("title", "x")]);
        page.properties
            .insert(RESERVED_SUPERTYPES_KEY.into(), strings(&["base:Thing"]));
        page.properties
            .insert(RESERVED_MIXINS_KEY.into(), strings(&["mix:Seo"]));
        upsert(&replica, &page, "a0", later(1_000)).await;
        assert_eq!(
            env.indexed("main", INDEXED_SUPERTYPE_KEY, "base:Thing", None)
                .await?,
            ["p"],
            "IS_A on a replica (skip={skip})"
        );
        assert_eq!(
            env.indexed("main", INDEXED_MIXIN_KEY, "mix:Seo", None)
                .await?,
            ["p"],
            "HAS_MIXIN on a replica (skip={skip})"
        );

        // A replicated update dropping the mixin retires its entry.
        page.properties
            .insert(RESERVED_MIXINS_KEY.into(), strings(&[]));
        upsert(&replica, &page, "a0", later(2_000)).await;
        assert!(env
            .indexed("main", INDEXED_MIXIN_KEY, "mix:Seo", None)
            .await?
            .is_empty());
        assert_eq!(
            env.indexed("main", INDEXED_SUPERTYPE_KEY, "base:Thing", None)
                .await?,
            ["p"]
        );
    }
    Ok(())
}

/// The out-of-order witness (`replication_out_of_order_test`) on a storage
/// whose local writes skip: the apply path keeps full puts, so r1 is written
/// in full for time travel and nothing of it stays live at HEAD.
#[tokio::test]
async fn replicated_op_older_than_head_does_not_leave_live_entries() -> Result<()> {
    let env = Env::new(true).await?;
    let replica = applicator(&env);
    env.add(
        "main",
        node("doc", "/doc", &[("title", "A"), ("keep", "k")]),
    )
    .await?;
    // A local skip-unchanged update first: `keep` stays at the create revision.
    env.put(
        "main",
        node("doc", "/doc", &[("title", "A2"), ("keep", "k")]),
    )
    .await?;

    let (r1, r2) = (later(60_000), later(120_000));
    upsert(&replica, &node("doc", "/doc", &[("title", "C")]), "a2", r2).await;
    upsert(
        &replica,
        &node("doc", "/doc", &[("title", "B"), ("keep", "k")]),
        "a1",
        r1,
    )
    .await;

    assert_eq!(env.indexed("main", "title", "C", None).await?, ["doc"]);
    for stale in ["B", "A2", "A"] {
        assert!(
            env.indexed("main", "title", stale, None).await?.is_empty(),
            "{stale} live at HEAD"
        );
    }
    assert!(
        env.indexed("main", "keep", "k", None).await?.is_empty(),
        "r1's `keep` is live at HEAD"
    );
    assert_eq!(env.indexed("main", "title", "B", Some(&r1)).await?, ["doc"]);
    assert_eq!(env.indexed("main", "keep", "k", Some(&r1)).await?, ["doc"]);
    Ok(())
}
