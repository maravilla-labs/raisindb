//! Plan Phase 13e review: workspace-owned compound indexes against a
//! declaration that changes while a build or a write is in flight, and the
//! paths whose node-type definitions are cold (they need none for the
//! workspace's own indexes).

use super::compound_env::item;
use super::env::{node, Env, REPO, TENANT, WS};
use super::replica_tests::{applicator, upsert};
use super::workspace_compound_tests::{page, ws_by_cat, WS_BY_CAT};
use crate::perf_counters::count;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::CompoundIndexDefinition;
use raisin_rocksdb::compound_state::CompoundStateStore;
use raisin_rocksdb::indexing::compound::workspace_defs;
use raisin_storage::compound::{CompoundBuildPhase, CompoundIndexState};

impl Env {
    /// `@by_cat` as the workspace declares it now (what a build job reads).
    fn ws_declared(&self) -> CompoundIndexDefinition {
        workspace_defs::current(self.storage.db(), TENANT, REPO, WS)
            .expect("declarations")
            .iter()
            .find(|d| d.name == WS_BY_CAT)
            .cloned()
            .expect("@by_cat declared")
    }

    /// A build of `definition` that registers and stamps (the clear and the
    /// pass in between change nothing about the stamp's decision).
    fn register_and_stamp(&self, definition: &CompoundIndexDefinition) -> Result<bool> {
        let store = CompoundStateStore::new(self.storage.db().clone());
        let floor = self.storage_head();
        let started = store.begin_rebuild(TENANT, REPO, "main", WS, definition, floor)?;
        store.complete_build(
            TENANT,
            REPO,
            "main",
            WS,
            CompoundIndexState::ready(definition, floor),
            started,
        )
    }

    fn storage_head(&self) -> HLC {
        HLC::new(chrono::Utc::now().timestamp_millis() as u64, 0)
    }

    fn invalidate_type_defs(&self) {
        raisin_rocksdb::indexing::compound::defs::invalidate_branch(
            self.storage.db(),
            TENANT,
            REPO,
            "main",
        );
    }
}

/// The FIRST build of a new declaration read it, then the declaration was
/// removed before the build registered: the reconcile found no record to
/// mark, so only the stamp itself can refuse. Brought back, the same
/// declaration must not find a `Ready` record nothing maintained.
#[tokio::test]
async fn a_declaration_removed_before_the_build_registered_never_stamps_ready() -> Result<()> {
    let env = Env::new(false).await?;
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    let read_by_the_job = env.ws_declared();
    env.declare(None).await?;
    assert!(env.ws_record("main").is_none(), "nothing to reconcile");

    assert!(!env.register_and_stamp(&read_by_the_job)?);
    let record = env.ws_record("main").expect("record");
    assert_eq!(record.phase, CompoundBuildPhase::NotBuilt);

    env.add("main", page("x", "a")).await?; // written while undeclared
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    assert!(!env.ws_ready("main"), "x was never indexed");
    env.build_compound("main").await?;
    assert_eq!(env.ws_listed("main", "a", None).await?, ["x"]);
    Ok(())
}

/// A `Ready` index's declaration changed between the job's read and its
/// registration: the mark advanced the generation BEFORE the build captured
/// it, so the compare-and-set alone would stamp the OLD hash. V1 → V2 → V1
/// must then not trust that record.
#[tokio::test]
async fn a_declaration_changed_before_the_build_registered_never_stamps_ready() -> Result<()> {
    let env = Env::new(false).await?;
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    env.build_compound("main").await?;
    assert!(env.ws_ready("main"));
    let read_by_the_job = env.ws_declared();
    env.declare(Some(vec![ws_by_cat(Some("tone"))])).await?;

    assert!(!env.register_and_stamp(&read_by_the_job)?);
    assert_eq!(
        env.ws_record("main").expect("record").phase,
        CompoundBuildPhase::NotBuilt
    );
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    assert!(
        !env.ws_ready("main"),
        "a V1 record stamped while V2 was declared was trusted again"
    );
    Ok(())
}

/// A transaction staged its workspace-index entries under V1; the
/// declaration changed to V2 and a build cleared, scanned (without the
/// uncommitted node) and stamped `Ready` before the transaction committed.
/// The commit must fail the index closed, not leave V1 tuples under a V2
/// `Ready`.
#[tokio::test]
async fn a_declaration_changed_while_a_write_was_staged_fails_the_index_closed() -> Result<()> {
    let env = Env::new(false).await?;
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    env.build_compound("main").await?;

    let tx = env.tx("main").await?;
    tx.put_node(WS, &node("x", "/x", &[("cat", "a"), ("tone", "warm")]))
        .await?;
    env.declare(Some(vec![ws_by_cat(Some("tone"))])).await?;
    env.build_compound("main").await?;
    assert!(env.ws_ready("main"), "V2 built before the commit");
    tx.commit().await?;

    assert!(
        !env.ws_ready("main"),
        "V1-layout entries committed under a V2 Ready record"
    );
    env.build_compound("main").await?;
    assert!(env.ws_ready("main"));
    Ok(())
}

/// A replicated upsert whose NodeType definitions are cold still maintains
/// the workspace's own index (it needs no NodeType) and leaves it `Ready`;
/// only the type's index is failed closed.
#[tokio::test]
async fn a_cold_replicated_write_keeps_the_workspace_index_ready() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    env.declare(Some(vec![ws_by_cat(None)])).await?;
    env.build_compound("main").await?;
    env.invalidate_type_defs();

    let at = HLC::new(chrono::Utc::now().timestamp_millis() as u64 + 1_000, 0);
    upsert(&applicator(&env), &item("x", "b", &[]), "a1", at).await;
    assert!(
        !env.compound_ready("main"),
        "the type's index needs its types"
    );
    assert!(
        env.ws_ready("main"),
        "one cold type took the listing offline"
    );
    assert_eq!(env.ws_listed("main", "b", None).await?, ["x"]);
    Ok(())
}

/// A delete with cold definitions derives the workspace index's tombstones
/// instead of scanning that keyspace — which holds every node of the
/// workspace — once per deleted node; and still ends the entry.
#[tokio::test]
async fn a_cold_delete_does_not_scan_the_workspace_index() -> Result<()> {
    use raisin_rocksdb::indexing::compound::tombstone_compound_for_delete;
    use raisin_rocksdb::indexing::IndexCtx;
    let ctx = IndexCtx::new(TENANT, REPO, "main", WS);
    let mut steps = Vec::new();
    for size in [20usize, 400] {
        let env = Env::new(false).await?;
        env.declare(Some(vec![ws_by_cat(None)])).await?;
        env.build_compound("main").await?;
        let tx = env.tx("main").await?;
        for i in 0..size {
            tx.add_node(WS, &page(&format!("n{i}"), "x")).await?;
        }
        tx.commit().await?;
        env.invalidate_type_defs();
        let old = page("n1", "x");
        let rev = HLC::new(u64::MAX / 2, 0);
        let ((), counts) = count(|| {
            let mut batch = rocksdb::WriteBatch::default();
            tombstone_compound_for_delete(&mut batch, env.storage.db(), &ctx, &old, &rev).unwrap();
        });
        steps.push(counts.next_on_memtable);

        if size == 20 {
            let r1 = env.newest_revision("main", "n2");
            let tx = env.tx("main").await?;
            tx.delete_node(WS, "n2").await?;
            tx.commit().await?;
            assert!(!env
                .ws_listed("main", "x", None)
                .await?
                .contains(&"n2".into()));
            assert!(env
                .ws_listed("main", "x", Some(&r1))
                .await?
                .contains(&"n2".into()));
            assert!(env.ws_ready("main"));
        }
    }
    assert!(
        steps[1] < steps[0] + 50,
        "a cold delete walked the workspace index: {steps:?}"
    );
    Ok(())
}
