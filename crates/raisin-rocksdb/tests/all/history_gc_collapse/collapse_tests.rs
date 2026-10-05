//! Run-collapse removes redundant versions and changes no answer at any
//! revision; it is idempotent, resumable, bounded by the watermark, and
//! refuses where it must.

use super::env::{node, options, Env, CFS, TENANT};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_rocksdb::cf;
use raisin_rocksdb::management::async_indexing::repair::{load_state, RepairOptions};

/// Twenty pages under a folder, each re-written four times with one field
/// changed: every unchanged property, membership and ordering entry is
/// re-put at each revision, which is the bloat collapse exists for.
async fn history(env: &Env) -> Result<Vec<HLC>> {
    let mut revs = vec![env.add("main", node("f", "/f", "folder")).await?];
    for i in 0..20 {
        let id = format!("p{i}");
        revs.push(
            env.add("main", node(&id, &format!("/f/{id}"), "v0"))
                .await?,
        );
    }
    for round in 1..=4 {
        for i in 0..20 {
            let id = format!("p{i}");
            let mut n = node(&id, &format!("/f/{id}"), "v0");
            n.properties.insert(
                "round".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(round.to_string()),
            );
            revs.push(env.put("main", n).await?);
        }
    }
    revs.push(env.delete("main", "p3").await?);
    Ok(revs)
}

fn versions(env: &Env, cf_name: &str) -> usize {
    env.raw(cf_name, "main").len()
}

#[tokio::test]
async fn collapse_preserves_every_index_read_at_every_revision() -> Result<()> {
    let env = Env::new().await?;
    let revs = history(&env).await?;
    env.prerequisites(Some("main")).await?;
    let before: Vec<_> = revs.iter().map(|r| env.decided_all("main", r)).collect();
    let prop_before = versions(&env, cf::PROPERTY_INDEX);

    let reports = env.collapse(Some("main"), options()).await?;
    let report = &reports[0];
    assert!(report.completed, "{report:?}");
    assert!(report.collapse.refused.is_empty(), "{report:?}");
    let prop = &report.collapse.column_families[cf::PROPERTY_INDEX];
    assert!(prop.deleted > 0, "{report:?}");
    // Each update re-puts the child's ordering entry (never skipped).
    assert!(
        report.collapse.column_families[cf::ORDERED_CHILDREN].deleted > 0,
        "{report:?}"
    );
    assert_eq!(
        versions(&env, cf::PROPERTY_INDEX),
        prop_before - prop.deleted as usize
    );

    for (rev, expected) in revs.iter().zip(&before) {
        assert_eq!(&env.decided_all("main", rev), expected, "at {rev}");
    }
    let state = load_state(
        env.storage.db(),
        TENANT,
        super::env::REPO,
        "main",
        "collapse_runs",
        "local",
    )?
    .expect("state record");
    assert_eq!(state.status, "done");
    Ok(())
}

#[tokio::test]
async fn collapse_idempotent() -> Result<()> {
    let env = Env::new().await?;
    history(&env).await?;
    env.prerequisites(Some("main")).await?;
    let first = env.collapse(Some("main"), options()).await?;
    assert!(first[0].writes.written > 0);
    let after: Vec<_> = CFS.iter().map(|(c, _)| env.raw(c, "main")).collect();

    let second = env.collapse(Some("main"), options()).await?;
    assert!(second[0].completed);
    assert_eq!(second[0].writes.written, 0, "{:?}", second[0].collapse);
    let again: Vec<_> = CFS.iter().map(|(c, _)| env.raw(c, "main")).collect();
    assert_eq!(after, again);
    Ok(())
}

#[tokio::test]
async fn collapse_ignores_entries_above_watermark() -> Result<()> {
    let env = Env::new().await?;
    let revs = history(&env).await?;
    env.prerequisites(Some("main")).await?;
    // Cap the watermark mid-history: nothing at or above it may go.
    let cap = revs[revs.len() / 2];
    let above = |env: &Env| -> Vec<Vec<(Vec<u8>, Vec<u8>)>> {
        CFS.iter()
            .map(|(c, tail)| {
                env.raw(c, "main")
                    .into_iter()
                    .filter(|(k, _)| super::env::split(k, *tail).is_some_and(|(_, r)| r >= cap))
                    .collect()
            })
            .collect()
    };
    let kept = above(&env);
    let before: Vec<_> = revs.iter().map(|r| env.decided_all("main", r)).collect();

    let reports = env
        .collapse(
            Some("main"),
            RepairOptions {
                collapse_watermark_cap: Some(cap),
                ..options()
            },
        )
        .await?;
    let counts = &reports[0].collapse;
    assert_eq!(counts.watermark.as_deref(), Some(cap.to_string().as_str()));
    assert!(counts.column_families[cf::PROPERTY_INDEX].deleted > 0);
    assert!(counts.column_families[cf::PROPERTY_INDEX].kept_above_watermark > 0);
    assert_eq!(above(&env), kept, "an entry at or above the watermark went");
    for (rev, expected) in revs.iter().zip(&before) {
        assert_eq!(&env.decided_all("main", rev), expected, "at {rev}");
    }
    Ok(())
}

#[tokio::test]
async fn collapse_dry_run_reports_bytes_and_deletes_nothing() -> Result<()> {
    let env = Env::new().await?;
    history(&env).await?;
    env.prerequisites(Some("main")).await?;
    let before: Vec<_> = CFS.iter().map(|(c, _)| env.raw(c, "main")).collect();
    let dry = env
        .collapse(
            Some("main"),
            RepairOptions {
                dry_run: true,
                ..options()
            },
        )
        .await?;
    let prop = &dry[0].collapse.column_families[cf::PROPERTY_INDEX];
    assert!(prop.deleted > 0 && prop.bytes_reclaimable > 0, "{prop:?}");
    let after: Vec<_> = CFS.iter().map(|(c, _)| env.raw(c, "main")).collect();
    assert_eq!(before, after);

    // The real run deletes exactly what the dry run predicted.
    let real = env.collapse(Some("main"), options()).await?;
    assert_eq!(
        real[0].collapse.column_families,
        dry[0].collapse.column_families
    );
    Ok(())
}

#[tokio::test]
async fn collapse_resumes_after_crash() -> Result<()> {
    // Two identical histories: one collapsed straight through, one stopped
    // after two small batches and resumed.
    let (straight, crashed) = (Env::new().await?, Env::new().await?);
    for env in [&straight, &crashed] {
        history(env).await?;
        env.prerequisites(Some("main")).await?;
    }
    straight.collapse(Some("main"), options()).await?;

    let small = || RepairOptions {
        batch_bytes: 512,
        ..options()
    };
    let stopped = crashed
        .collapse(
            Some("main"),
            RepairOptions {
                stop_after_batches: Some(2),
                ..small()
            },
        )
        .await?;
    assert!(!stopped[0].completed);
    let resumed = crashed.collapse(Some("main"), small()).await?;
    assert!(
        resumed[0].resumed && resumed[0].completed,
        "{:?}",
        resumed[0]
    );
    assert!(
        resumed[0].writes.batches > 1,
        "the run must actually be bounded"
    );

    for (c, _) in CFS {
        assert_eq!(
            straight.raw(c, "main").len(),
            crashed.raw(c, "main").len(),
            "{c}: the resumed run must end where the uninterrupted one does"
        );
    }
    Ok(())
}

#[tokio::test]
async fn collapse_disk_precheck_refuses_without_headroom() -> Result<()> {
    let env = Env::new().await?;
    history(&env).await?;
    env.prerequisites(Some("main")).await?;
    let db = env.storage.db();
    db.flush_cf(db.cf_handle(cf::PROPERTY_INDEX).unwrap())
        .unwrap();
    let before = env.raw(cf::PROPERTY_INDEX, "main");
    let err = env
        .collapse(
            Some("main"),
            RepairOptions {
                check_headroom: true,
                free_bytes_override: Some(1),
                ..options()
            },
        )
        .await
        .expect_err("no headroom");
    assert!(err.to_string().contains("free"), "{err}");
    assert_eq!(env.raw(cf::PROPERTY_INDEX, "main"), before);
    Ok(())
}

#[tokio::test]
async fn collapse_refuses_in_cluster_mode_and_when_disabled() -> Result<()> {
    let env = Env::new().await?;
    history(&env).await?;
    env.prerequisites(Some("main")).await?;
    let err = env
        .collapse(
            Some("main"),
            RepairOptions {
                cluster_mode: true,
                ..options()
            },
        )
        .await
        .expect_err("cluster mode has no watermark");
    assert!(err.to_string().contains("watermark"), "{err}");

    let off = Env::with_flag(false).await?;
    let err = off
        .collapse(Some("main"), options())
        .await
        .expect_err("disabled by default");
    assert!(err.to_string().contains("disabled"), "{err}");
    Ok(())
}

#[tokio::test]
async fn collapse_waits_out_an_inserter_then_reports_busy() -> Result<()> {
    let env = Env::new().await?;
    history(&env).await?;
    env.prerequisites(Some("main")).await?;
    let before = env.raw(cf::PROPERTY_INDEX, "main");
    let held = raisin_rocksdb::management::cf_exclusion::enter_inserter(
        env.storage.db(),
        TENANT,
        super::env::REPO,
        "main",
        cf::PROPERTY_INDEX,
    );
    let busy = env
        .collapse(
            Some("main"),
            RepairOptions {
                collapse_cfs: Some(vec![cf::PROPERTY_INDEX.to_string()]),
                ..options()
            },
        )
        .await?;
    assert!(!busy[0].completed);
    assert_eq!(busy[0].collapse.busy, vec![cf::PROPERTY_INDEX.to_string()]);
    assert_eq!(env.raw(cf::PROPERTY_INDEX, "main"), before);
    drop(held);
    let done = env.collapse(Some("main"), options()).await?;
    assert!(done[0].completed);
    assert!(env.raw(cf::PROPERTY_INDEX, "main").len() < before.len());
    Ok(())
}
