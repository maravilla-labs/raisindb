//! Phase 8 review regressions for merges: a source rebuilt after its
//! changes, a fork merged back without one, and the target's definitions
//! cache after the merge copied NodeType versions.

use super::compound_env::{by_cat, item, ITEM};
use super::compound_review_tests::by_title;
use super::env::{Env, REPO, TENANT};
use super::node_types::register_type_with;
use raisin_context::MergeStrategy;
use raisin_error::Result;
use raisin_rocksdb::indexing::compound::defs;
use raisin_storage::BranchScope;

impl Env {
    async fn merge_into_main(&self, source: &str) -> Result<()> {
        let result = self
            .storage
            .branches_impl()
            .merge_branches(
                TENANT,
                REPO,
                "main",
                source,
                MergeStrategy::ThreeWay,
                "merge",
                "test-user",
            )
            .await?;
        assert!(result.success && result.conflicts.is_empty());
        Ok(())
    }
}

/// The source rebuilt after it changed `x`: its keyspace no longer holds the
/// tombstone that ended the target's tuple, so the merge cannot replay it —
/// the target's index is failed closed and rebuilt, never left matching both.
#[tokio::test]
async fn merge_after_a_source_rebuild_does_not_leave_the_target_tuple_live() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    env.add("main", item("x", "a", &[])).await?;
    env.fork("feature").await?;
    env.put("feature", item("x", "c", &[])).await?;
    env.build_compound("feature").await?;
    env.add("main", item("y", "z", &[])).await?;
    env.merge_into_main("feature").await?;
    assert!(
        !env.compound_ready("main"),
        "main would match x under both `a` and `c`"
    );
    env.build_compound("main").await?;
    assert_eq!(env.compound("main", "c", None).await?, ["x"]);
    assert!(env.compound("main", "a", None).await?.is_empty());
    Ok(())
}

/// Without a rebuild the source's writers' tombstones replay into the target:
/// the fork inherited `Ready` with the copied entries, and the merge keeps
/// the target `Ready` and correct.
#[tokio::test]
async fn merge_from_an_unrebuilt_fork_keeps_the_target_ready() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    env.add("main", item("x", "a", &[])).await?;
    env.fork("feature").await?;
    assert!(env.compound_ready("feature"), "a fork inherits Ready");
    env.put("feature", item("x", "c", &[])).await?;
    env.add("main", item("y", "z", &[])).await?;
    env.merge_into_main("feature").await?;
    assert!(env.compound_ready("main"));
    assert_eq!(env.compound("main", "c", None).await?, ["x"]);
    assert!(env.compound("main", "a", None).await?.is_empty());
    Ok(())
}

/// A merge copies the source's NodeType versions; the target's definitions
/// cache (cache-first, feeding every write and build there) must follow.
#[tokio::test]
async fn merge_refreshes_the_target_definitions_cache() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    env.add("main", item("x", "a", &[])).await?;
    env.fork("feature").await?;
    register_type_with(
        &env.storage,
        "feature",
        ITEM,
        Some("code"),
        None,
        Some(vec![by_cat(), by_title()]),
    )
    .await?;
    env.add("main", item("y", "z", &[])).await?;
    env.merge_into_main("feature").await?;
    let main = BranchScope::new(TENANT, REPO, "main");
    let cached = defs::peek(env.storage.db(), main, ITEM).expect("cached");
    assert!(
        cached.compound.iter().any(|d| d.name == "by_title"),
        "main still writes under the pre-merge declaration: {:?}",
        cached.compound
    );
    Ok(())
}
