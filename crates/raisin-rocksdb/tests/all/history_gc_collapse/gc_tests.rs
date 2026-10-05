//! Retention GC and run-collapse together, against reorders and a second
//! merge; the shared GC helpers (`gc_copy_tests` uses them too).

use super::env::{node, options, Env, REPO, TENANT, WS};
use raisin_context::MergeStrategy;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_rocksdb::cf;
use raisin_rocksdb::management::history_gc::{run_history_gc, GcOptions, HistoryRetention};
use raisin_storage::{NodeRepository, PropertyIndexRepository, Storage};
use std::time::Duration;

pub(super) fn keep(n: u64) -> GcOptions {
    GcOptions {
        retention_override: Some(HistoryRetention {
            keep_days: None,
            keep_revisions: Some(n),
        }),
        min_age: Duration::ZERO,
        collect_orphaned_blobs: false,
        bound_job_results: false,
        sweep_unreferenced_blobs: false,
        compact: false,
        ..GcOptions::default()
    }
}

/// Retention GC, then the prerequisites and collapse on every branch.
pub(super) async fn gc_and_collapse(env: &Env, gc: GcOptions) -> Result<()> {
    run_history_gc(&env.storage, &gc)?;
    env.prerequisites(None).await?;
    let reports = env.collapse(None, options()).await?;
    assert!(reports.iter().all(|r| r.completed), "{reports:?}");
    Ok(())
}

pub(super) async fn title(
    env: &Env,
    branch: &str,
    id: &str,
    at: Option<&HLC>,
) -> Result<Option<String>> {
    Ok(env
        .storage
        .nodes()
        .get(env.scope(branch), id, at)
        .await?
        .and_then(|n| match n.properties.get("title") {
            Some(PropertyValue::String(s)) => Some(s.clone()),
            _ => None,
        }))
}

pub(super) async fn titled(env: &Env, branch: &str, value: &str) -> Result<Vec<String>> {
    env.storage
        .property_index()
        .find_by_property(
            env.scope(branch),
            "title",
            &PropertyValue::String(value.to_string()),
            false,
            None,
        )
        .await
}

/// The children listed under `parent` at HEAD, in editorial order, straight
/// from ORDERED_CHILDREN (a group key sorts by label).
fn children(env: &Env, parent: &str) -> Vec<String> {
    let head_prefix =
        raisin_rocksdb::keys::ordered_children_prefix(TENANT, REPO, "main", WS, parent);
    env.decided(cf::ORDERED_CHILDREN, "main", &HLC::new(u64::MAX, u64::MAX))
        .keys()
        .filter(|g| g.starts_with(&head_prefix))
        .map(|g| {
            let at = g.iter().rposition(|b| *b == 0).unwrap();
            String::from_utf8_lossy(&g[at + 1..]).into_owned()
        })
        .collect()
}

#[tokio::test]
async fn gc_keeps_reordered_child_hidden_in_old_group() -> Result<()> {
    let env = Env::new().await?;
    env.add("main", node("f", "/f", "folder")).await?;
    for id in ["a", "b", "c"] {
        env.add("main", node(id, &format!("/f/{id}"), id)).await?;
    }
    for v in 1..=2 {
        env.put("main", node("c", "/f/c", &format!("c{v}"))).await?;
    }
    env.storage
        .nodes()
        .move_child_before(env.scope("main"), "/f", "c", "a", None, None)
        .await?;
    env.put("main", node("c", "/f/c", "c3")).await?;
    assert_eq!(children(&env, "f"), ["c", "a", "b"]);

    gc_and_collapse(
        &env,
        GcOptions {
            retention_override: Some(HistoryRetention {
                keep_days: Some(0),
                keep_revisions: None,
            }),
            ..keep(1)
        },
    )
    .await?;

    // The old label's tombstone survives both: c is listed once, first.
    assert_eq!(children(&env, "f"), ["c", "a", "b"]);
    let listed: Vec<String> = env
        .storage
        .nodes()
        .list_children(env.scope("main"), "/f", Default::default())
        .await?
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(listed, ["c", "a", "b"]);
    Ok(())
}

#[tokio::test]
async fn gc_keeps_merge_base_after_second_merge() -> Result<()> {
    let env = Env::new().await?;
    env.add("main", node("doc", "/doc", "v0")).await?;
    env.add("main", node("other", "/other", "o0")).await?;
    env.fork("feature", "main").await?;
    env.put("feature", node("doc", "/doc", "f1")).await?;
    env.put("main", node("other", "/other", "o1")).await?;
    let branches = env.storage.branches_impl();
    let merge = |label: &'static str| async move {
        branches
            .merge_branches(
                TENANT,
                REPO,
                "main",
                "feature",
                MergeStrategy::ThreeWay,
                label,
                "test-user",
            )
            .await
    };
    let first = merge("first").await?;
    assert!(first.success && first.conflicts.is_empty(), "{first:?}");
    env.put("feature", node("doc", "/doc", "f2")).await?;
    env.put("main", node("other", "/other", "o2")).await?;

    let base = env
        .storage
        .branches()
        .calculate_divergence(TENANT, REPO, "main", "feature")
        .await?
        .common_ancestor;
    let base_title = title(&env, "feature", "doc", Some(&base)).await?;
    assert_eq!(base_title.as_deref(), Some("f1"));

    gc_and_collapse(&env, keep(1)).await?;

    // The merge base still reads as it did: GC pinned it.
    assert_eq!(
        title(&env, "feature", "doc", Some(&base)).await?,
        base_title
    );
    let second = merge("second").await?;
    assert!(second.success && second.conflicts.is_empty(), "{second:?}");
    assert_eq!(
        title(&env, "main", "doc", None).await?.as_deref(),
        Some("f2")
    );
    assert_eq!(
        title(&env, "main", "other", None).await?.as_deref(),
        Some("o2")
    );
    Ok(())
}
