//! Retention GC and run-collapse, then operations that read retained
//! history back: a cross-branch copy at a pinned revision, a restore, and
//! forks collected independently.

use super::env::{node, options, Env, CFS, REPO, TENANT, WS};
use super::gc_tests::{gc_and_collapse, keep, title, titled};
use raisin_error::Result;
use raisin_rocksdb::cf;
use raisin_rocksdb::management::history_gc::{
    retention, run_history_gc, GcOptions, HistoryRetention,
};
use raisin_storage::{NodeRepository, Storage, TagRepository};

#[tokio::test]
async fn gc_then_cross_branch_copy_at_pinned_revision() -> Result<()> {
    let env = Env::new().await?;
    env.add("main", node("docs", "/docs", "docs")).await?;
    env.add("main", node("a", "/docs/a", "v1")).await?;
    env.fork("publish", "main").await?;
    let tagged = env.put("main", node("a", "/docs/a", "v2")).await?;
    env.storage
        .tags()
        .create_tag(TENANT, REPO, "release", &tagged, "test-user", None, false)
        .await?;
    for v in 3..=4 {
        env.put("main", node("a", "/docs/a", &format!("v{v}")))
            .await?;
    }

    gc_and_collapse(&env, keep(1)).await?;

    env.storage
        .nodes()
        .copy_nodes_across_branches(
            TENANT,
            REPO,
            "main",
            "publish",
            WS,
            &["/docs/a".to_string()],
            false,
            false,
            Some(&tagged),
            None,
        )
        .await?;
    assert_eq!(
        title(&env, "publish", "a", None).await?.as_deref(),
        Some("v2")
    );
    assert_eq!(titled(&env, "publish", "v2").await?, vec!["a".to_string()]);
    assert!(titled(&env, "publish", "v1").await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn gc_then_restore_to_retained_revision() -> Result<()> {
    let env = Env::new().await?;
    let mut revs = Vec::new();
    for v in 1..=6 {
        revs.push(
            env.put("main", node("page", "/page", &format!("v{v}")))
                .await?,
        );
    }
    gc_and_collapse(&env, keep(3)).await?;

    // v4 is inside the window: read it back and write it as the new HEAD,
    // which is what a restore does.
    let retained = env
        .storage
        .nodes()
        .get(env.scope("main"), "page", Some(&revs[3]))
        .await?
        .expect("a retained revision still reads");
    assert_eq!(
        title(&env, "main", "page", Some(&revs[3]))
            .await?
            .as_deref(),
        Some("v4")
    );
    env.put("main", retained).await?;

    assert_eq!(
        title(&env, "main", "page", None).await?.as_deref(),
        Some("v4")
    );
    assert_eq!(titled(&env, "main", "v4").await?, vec!["page".to_string()]);
    assert!(titled(&env, "main", "v6").await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn gc_on_fork_independent() -> Result<()> {
    let env = Env::new().await?;
    for v in 1..=3 {
        env.put("main", node("page", "/page", &format!("m{v}")))
            .await?;
    }
    env.fork("feature", "main").await?;
    let mut feature_revs = Vec::new();
    for v in 1..=4 {
        env.put("main", node("page", "/page", &format!("m{}", v + 3)))
            .await?;
        feature_revs.push(
            env.put("feature", node("page", "/page", &format!("f{v}")))
                .await?,
        );
    }
    let all_cfs = |env: &Env, branch: &str| -> Vec<Vec<(Vec<u8>, Vec<u8>)>> {
        CFS.iter()
            .map(|(c, _)| env.raw(c, branch))
            .chain(std::iter::once(env.raw(cf::NODES, branch)))
            .collect()
    };
    let main_before = all_cfs(&env, "main");
    let feature_reads: Vec<_> = feature_revs
        .iter()
        .map(|r| env.decided_all("feature", r))
        .collect();

    // Retention on the fork only, then collapse on the fork only.
    retention::set_policy(
        env.storage.db(),
        TENANT,
        REPO,
        "feature",
        Some(&HistoryRetention {
            keep_days: None,
            keep_revisions: Some(2),
        }),
    )?;
    run_history_gc(
        &env.storage,
        &GcOptions {
            retention_override: None,
            ..keep(2)
        },
    )?;
    env.prerequisites(Some("feature")).await?;
    let reports = env.collapse(Some("feature"), options()).await?;
    assert!(reports[0].completed);
    assert!(reports[0].writes.written > 0, "{:?}", reports[0].collapse);

    assert_eq!(all_cfs(&env, "main"), main_before, "main changed");
    // The fork's retained revisions (the last two) read as before.
    for (rev, expected) in feature_revs.iter().zip(&feature_reads).skip(2) {
        assert_eq!(&env.decided_all("feature", rev), expected, "at {rev}");
    }

    // And the other way round.
    let feature_now = all_cfs(&env, "feature");
    env.prerequisites(Some("main")).await?;
    env.collapse(Some("main"), options()).await?;
    assert_eq!(all_cfs(&env, "feature"), feature_now, "the fork changed");
    Ok(())
}
