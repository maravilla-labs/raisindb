//! Plan Phase 7b: `index.skip_unchanged` is ON by default, still gated per
//! branch on this node's `property_index` rebuild — and that rebuild queues
//! itself, per branch, in the background (the `node_path` backfill's chain).

use super::env::{repair_options, Env, REPO, TENANT};
use raisin_error::Result;
use raisin_rocksdb::management::async_indexing::repair::{
    continue_chain, pending_property_index_branches, register_requester, run_repair, start_chain,
    RepairKind,
};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::jobs::{JobContext, JobType};
use raisin_storage::{BranchRepository, Storage};
use std::time::Duration;

/// The branches of the test repository with a queued `property_index` job,
/// with each job's context, in branch order.
async fn queued(storage: &RocksDBStorage) -> Vec<(String, JobContext)> {
    let mut out: Vec<(String, JobContext)> = storage
        .job_registry()
        .list_jobs_by_tenant(TENANT)
        .await
        .into_iter()
        .filter_map(|job| match &job.job_type {
            JobType::IndexRepair {
                repo_id,
                branch: Some(branch),
                repair,
                dry_run: false,
                ..
            } if repo_id == REPO && repair == RepairKind::PropertyIndex.slug() => {
                let context = storage
                    .job_data_store()
                    .get(TENANT, &job.id)
                    .unwrap()
                    .expect("job context");
                Some((branch.clone(), context))
            }
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn pending(storage: &RocksDBStorage) -> Result<Vec<String>> {
    Ok(pending_property_index_branches(storage)?
        .into_iter()
        .filter(|(t, r, _)| t == TENANT && r == REPO)
        .map(|(_, _, b)| b)
        .collect())
}

#[tokio::test]
async fn skip_unchanged_is_on_by_default() -> Result<()> {
    if std::env::var("RAISIN_INDEX_SKIP_UNCHANGED").is_ok() {
        return Ok(()); // the environment overrides the default either way
    }
    let dir = tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
    let storage = RocksDBStorage::new(dir.path())?;
    assert!(storage.nodes_impl().index_skip_unchanged());
    assert!(raisin_rocksdb::RocksDBConfig::default().index_skip_unchanged);
    Ok(())
}

/// Every branch not rebuilt on this node is pending; the start queues ONE
/// job (for the first), each finished link queues the next, and a rebuilt
/// branch is no longer pending. With the flag off nothing is pending at all
/// (a rebuild would unlock nothing).
#[tokio::test]
async fn unrebuilt_branches_get_the_rebuild_queued_branch_by_branch() -> Result<()> {
    let env = Env::new(false).await?;
    let storage = &env.storage;
    env.fork("a-fork").await?;
    assert!(pending(storage)?.is_empty(), "flag off: nothing to unlock");
    assert_eq!(start_chain(storage, RepairKind::PropertyIndex).await?, 0);

    storage.nodes_impl().set_index_skip_unchanged(true);
    assert_eq!(pending(storage)?, ["a-fork", "main"]);
    assert_eq!(start_chain(storage, RepairKind::PropertyIndex).await?, 1);
    let jobs = queued(storage).await;
    assert_eq!(
        jobs.iter().map(|j| j.0.as_str()).collect::<Vec<_>>(),
        ["a-fork"]
    );
    assert_eq!(
        start_chain(storage, RepairKind::PropertyIndex).await?,
        0,
        "one link at a time"
    );

    // The link's work (the job handler runs exactly this), then the chain
    // moves on to the next pending branch.
    run_repair(
        storage,
        TENANT,
        REPO,
        Some("a-fork"),
        RepairKind::PropertyIndex,
        repair_options(),
    )
    .await?;
    assert_eq!(pending(storage)?, ["main"]);
    let first = jobs[0].1.clone();
    let chained = continue_chain(
        storage,
        RepairKind::PropertyIndex,
        &first,
        TENANT,
        REPO,
        Some("a-fork"),
    );
    assert_eq!(chained.await?, 1);
    assert_eq!(
        queued(storage)
            .await
            .iter()
            .map(|j| j.0.as_str())
            .collect::<Vec<_>>(),
        ["a-fork", "main"]
    );
    env.rebuild("main").await?;
    assert!(pending(storage)?.is_empty());
    Ok(())
}

/// A fork made while the job system runs asks for its own rebuild once its
/// index copy is done — it does not wait for the next start.
#[tokio::test]
async fn a_fork_requests_its_rebuild() -> Result<()> {
    let env = Env::new(true).await?;
    register_requester(&env.storage);
    env.storage
        .branches()
        .create_branch(
            TENANT,
            REPO,
            "draft",
            "test-user",
            None,
            Some("main".to_string()),
            false,
            false,
        )
        .await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if queued(&env.storage).await.iter().any(|(b, _)| b == "draft") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no property_index rebuild was queued for the fork"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}
