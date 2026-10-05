//! Plan Phase 13f (owner decision 2026-10-05): the automatic rebuild of
//! older-format compound indexes by the `compound_builds` repair, and its
//! switch `RAISIN_COMPOUND_FORMAT_REBUILD`.

use super::compound_env::{by_cat, item, ITEM};
use super::env::{Env, REPO, TENANT, WS};
use super::node_types::register_type_with;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_rocksdb::compound_state::CompoundStateStore;
use raisin_rocksdb::management::async_indexing::repair::{start_chain, RepairKind};
use raisin_storage::compound::CompoundIndexState;

/// Owner decision (2026-10-05): an index whose state record is an older
/// FORMAT is rebuilt AUTOMATICALLY — the branch is owed, the chain started
/// after boot queues its link, and the link (what the job runs) makes it
/// `Ready` with no admin call. `RAISIN_COMPOUND_FORMAT_REBUILD=0` leaves it
/// alone. One test: the switch is process-wide.
#[tokio::test]
async fn format_upgrade_is_rebuilt_automatically_unless_switched_off() -> Result<()> {
    let env = Env::new(false).await?;
    env.builtin(false).await?; // only by_cat in play
    register_type_with(&env.storage, "main", ITEM, None, None, Some(vec![by_cat()])).await?;
    env.add("main", item("i", "a", &[])).await?;
    let store = CompoundStateStore::new(env.storage.db().clone());
    let old_format = || {
        let mut v1 = CompoundIndexState::ready(&by_cat(), HLC::new(1, 0));
        v1.v = CompoundIndexState::VERSION - 1;
        v1
    };

    // Switched off: nothing owed, the link leaves it, no sweep queues it.
    raisin_rocksdb::compound_state::override_format_rebuild(Some(false));
    let outcome = async {
        store.put(TENANT, REPO, "main", WS, &old_format())?;
        env.compound_builds("main").await?; // clears the "no record" owing
        assert!(env.pending_compound_builds().await?.is_empty());
        assert_eq!(
            env.storage
                .sweep_compound_index_builds(TENANT, REPO, "main", WS)
                .await?,
            0
        );
        assert!(!env.compound_ready("main"), "left to an admin rebuild");

        // Default (on): owed, the started chain queues the link, the link
        // rebuilds it.
        raisin_rocksdb::compound_state::override_format_rebuild(None);
        if !raisin_rocksdb::compound_state::format_rebuild_enabled() {
            return Ok(()); // the environment switched it off for this run
        }
        assert_eq!(env.pending_compound_builds().await?, ["main"]);
        assert_eq!(
            env.storage
                .sweep_compound_index_builds(TENANT, REPO, "main", WS)
                .await?,
            0,
            "never a per-index job: the repair owns it"
        );
        assert_eq!(
            start_chain(&env.storage, RepairKind::CompoundBuilds).await?,
            1
        );
        let report = env.compound_builds("main").await?;
        assert_eq!(report.compound.built, 1);
        assert!(env.compound_ready("main"));
        assert_eq!(env.compound("main", "a", None).await?, ["i"]);
        assert!(env.pending_compound_builds().await?.is_empty());
        Ok::<(), raisin_error::Error>(())
    }
    .await;
    raisin_rocksdb::compound_state::override_format_rebuild(None);
    outcome
}
