//! Regression tests for the Phase 8 review findings (compound builds' history
//! floor, merges after a rebuild, concurrent writers, the definitions cache,
//! types with no NodeType on a replica, unplaceable nodes, format upgrades).

use super::compound_env::{by_cat, item, BY_CAT, ITEM};
use super::env::{Env, REPO, TENANT, WS};
use super::node_types::register_type_with;
use super::replica_tests::{applicator, upsert};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
};
use raisin_rocksdb::compound_state::{read_state, CompoundStateStore};
use raisin_rocksdb::indexing::compound::{cold, defs};
use raisin_rocksdb::{cf, keys};
use raisin_storage::compound::CompoundIndexState;
use raisin_storage::{BranchScope, Storage};

fn later(offset_ms: u64) -> HLC {
    HLC::new(chrono::Utc::now().timestamp_millis() as u64 + offset_ms, 0)
}

pub(super) fn by_title() -> CompoundIndexDefinition {
    CompoundIndexDefinition {
        name: "by_title".to_string(),
        columns: vec![CompoundIndexColumn {
            property: "title".to_string(),
            ascending: None,
            column_type: CompoundColumnType::String,
        }],
        has_order_column: false,
        owner: None,
    }
}

impl Env {
    pub(super) fn compound_ready_at(&self, branch: &str, at: Option<&HLC>) -> bool {
        self.storage
            .compound_state()
            .expect("compound state source")
            .compound_availability(TENANT, REPO, branch, WS, &by_cat())
            .at_revision(at)
            .is_ready()
    }
}

/// A rebuild re-derives the index as of HEAD and keeps no history below it:
/// time travel to a revision before the rebuild must not be index-served.
#[tokio::test]
async fn time_travel_below_a_rebuild_is_not_index_served() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    env.add("main", item("x", "a", &[])).await?;
    let r1 = env.newest_revision("main", "x");
    env.put("main", item("x", "b", &[])).await?;
    let r2 = env.newest_revision("main", "x");
    env.build_compound("main").await?;

    let state = read_state(env.storage.db(), TENANT, REPO, "main", WS, BY_CAT)?.unwrap();
    assert!(state.built_through >= r2, "the floor is the HEAD it read");
    assert!(env.compound_ready_at("main", None));
    assert!(env.compound_ready_at("main", Some(&state.built_through)));
    assert!(
        !env.compound_ready_at("main", Some(&r1)),
        "a read at r1 must scan: the rebuild kept no history below its floor"
    );
    assert_eq!(env.compound("main", "b", None).await?, ["x"]);
    assert!(env.compound("main", "a", None).await?.is_empty());
    Ok(())
}

/// Default config (no skip): two writers both read `a` and each ends only
/// `a`. The commit-time re-check must run anyway, or the first committer's
/// tuple stays live beside the second's.
#[tokio::test]
async fn concurrent_updates_with_skip_off_end_the_losing_tuple() -> Result<()> {
    for b_first in [false, true] {
        let env = Env::new(false).await?;
        env.with_items("main").await?;
        env.add("main", item("x", "a", &[])).await?;
        let tx_b = env.tx("main").await?;
        tx_b.put_node(WS, &item("x", "b", &[])).await?;
        let tx_c = env.tx("main").await?;
        tx_c.put_node(WS, &item("x", "c", &[])).await?;
        if b_first {
            tx_b.commit().await?;
            tx_c.commit().await?;
        } else {
            tx_c.commit().await?;
            tx_b.commit().await?;
        }
        // `c` was staged at the later revision: it is the newest version.
        assert_eq!(env.compound("main", "c", None).await?, ["x"], "{b_first}");
        assert!(
            env.compound("main", "b", None).await?.is_empty(),
            "{b_first}"
        );
        assert!(
            env.compound("main", "a", None).await?.is_empty(),
            "{b_first}"
        );
    }
    Ok(())
}

/// An ancestor move re-keys a descendant's compound entries without a NODES
/// version. A descendant write staged before it and committed after cannot be
/// corrected from NODES: the index must be failed closed, not left wrong.
/// (The move keeps `p`'s name, so `x` — whose record holds its parent's NAME —
/// is re-keyed, not rewritten; a rename rewrites `x`, and that record write
/// is corrected from NODES like any racing version.)
#[tokio::test]
async fn concurrent_ancestor_move_fails_the_compound_index_closed() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    env.add("main", super::env::node("p", "/p", &[])).await?;
    env.add("main", super::env::node("q", "/q", &[])).await?;
    let mut x = item("x", "a", &[]);
    x.path = "/p/x".to_string();
    x.parent = Some("p".to_string());
    env.add("main", x.clone()).await?;
    assert!(env.compound_ready("main"));

    let tx_write = env.tx("main").await?;
    x.properties.insert(
        "cat".into(),
        raisin_models::nodes::properties::PropertyValue::String("b".into()),
    );
    tx_write.put_node(WS, &x).await?;
    let tx_move = env.tx("main").await?;
    tx_move.move_node_tree(WS, "p", "/q/p").await?;
    tx_move.commit().await?;
    tx_write.commit().await?;
    assert!(
        !env.compound_ready("main"),
        "a concurrent index-only write must fail the index closed"
    );
    assert!(cold::is_requested(
        env.storage.db(),
        TENANT,
        REPO,
        "main",
        WS
    ));
    Ok(())
}

/// A node type with no NodeType record on the replica's branch is listed by
/// nothing: one cold write must be enough to cache it, or every later write
/// of such a node marks the workspace and rebuilds it.
#[tokio::test]
async fn replica_type_without_nodetype_turns_warm_after_one_cold_write() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    let replica = applicator(&env);
    let mut ghost = item("g", "x", &[]);
    ghost.node_type = "legacy:Ghost".to_string();
    upsert(&replica, &ghost, "a1", later(1_000)).await;
    assert!(!env.compound_ready("main"), "the first write is cold");

    // What the job event handler's drain does: resolve the requested types
    // (and the branch), then build.
    let requests = cold::drain(env.storage.db());
    let types: Vec<&str> = requests
        .iter()
        .flat_map(|(_, types)| types.iter().map(String::as_str))
        .collect();
    assert!(types.contains(&"legacy:Ghost"), "{types:?}");
    let main = BranchScope::new(TENANT, REPO, "main");
    defs::fresh_branch(env.storage.db(), env.storage.node_types(), main, &types).await?;
    env.build_compound("main").await?;
    assert!(env.compound_ready("main"));

    upsert(&replica, &ghost, "a1", later(2_000)).await;
    assert!(
        env.compound_ready("main"),
        "a type with no NodeType must stay warm after the drain"
    );
    Ok(())
}

/// A node the rebuild cannot place: refuse BEFORE the clear, so the entries
/// that exist survive and nothing is stamped over a hole.
#[tokio::test]
async fn rebuild_refuses_before_clearing_when_a_node_cannot_be_placed() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    env.add("main", item("x", "a", &[])).await?;
    let db = env.storage.db();
    let cf_path = db.cf_handle(cf::NODE_PATH).unwrap();
    let prefix = keys::node_path_key_prefix(TENANT, REPO, "main", WS, "x");
    let doomed: Vec<Box<[u8]>> = db
        .iterator_cf(
            cf_path,
            rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
        )
        .map(|item| item.unwrap().0)
        .take_while(|key| key.starts_with(&prefix))
        .collect();
    assert!(!doomed.is_empty());
    for key in doomed {
        db.delete_cf(cf_path, key).unwrap();
    }
    assert!(env.build_compound("main").await.is_err());
    assert_eq!(
        env.compound("main", "a", None).await?,
        ["x"],
        "the refused rebuild must not have cleared the keyspace"
    );
    assert!(env.compound_ready("main"));
    Ok(())
}

/// An index whose record is merely an older FORMAT is never a per-index
/// sweep job (at boot that would be every index on every node at once): the
/// `compound_builds` repair owns it (plan Phase 13f,
/// `compound_format_tests::format_upgrade_is_rebuilt_automatically_unless_switched_off`).
/// One that is genuinely stale still is.
#[tokio::test]
async fn format_upgrade_is_left_to_an_admin_rebuild() -> Result<()> {
    let env = Env::new(false).await?;
    register_type_with(&env.storage, "main", ITEM, None, None, Some(vec![by_cat()])).await?;
    let store = CompoundStateStore::new(env.storage.db().clone());
    let mut v1 = CompoundIndexState::ready(&by_cat(), HLC::new(1, 0));
    v1.v = CompoundIndexState::VERSION - 1;
    store.put(TENANT, REPO, "main", WS, &v1)?;
    assert!(!env.compound_ready("main"));
    let queued = env
        .storage
        .sweep_compound_index_builds(TENANT, REPO, "main", WS)
        .await?;
    assert_eq!(
        queued, 0,
        "a format upgrade is the compound_builds repair's"
    );
    let mut stale = CompoundIndexState::ready(&by_cat(), HLC::new(1, 0));
    stale.phase = raisin_storage::compound::CompoundBuildPhase::NotBuilt;
    store.put(TENANT, REPO, "main", WS, &stale)?;
    let queued = env
        .storage
        .sweep_compound_index_builds(TENANT, REPO, "main", WS)
        .await?;
    assert_eq!(queued, 1, "a stale index is still swept");
    Ok(())
}
