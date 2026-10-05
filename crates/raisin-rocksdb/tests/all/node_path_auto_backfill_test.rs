//! Phase 10b: existing databases migrate without an admin call.
//!
//! After the job system starts, the branches whose `node_path` backfill has
//! not completed on this node are backfilled by ordinary `IndexRepair` jobs —
//! the same streaming, resumable, disk-checked run the admin endpoint starts —
//! ONE branch at a time: each job of the automatic chain queues the next
//! pending branch when it finishes.

use crate::node_path_writer_test::{
    backfill_options, folder, legacy_tx_put, node_path_entries, setup, BRANCH, REPO, TENANT,
};
use raisin_error::Result;
use raisin_rocksdb::management::async_indexing::repair::{
    continue_node_path_backfill_chain, enqueue_pending_node_path_backfills,
    pending_node_path_branches, run_repair, RepairKind,
};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::jobs::{JobContext, JobType};
use raisin_storage::{BranchRepository, Storage};

fn is_main(entry: &(String, String, String)) -> bool {
    entry.0 == TENANT && entry.1 == REPO && entry.2 == BRANCH
}

/// The branches with a queued (non-dry-run) `node_path` job, with each job's
/// context, in branch order.
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
            } if repo_id == REPO && repair == RepairKind::NodePath.slug() => {
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

#[tokio::test]
async fn pending_branches_get_the_node_path_backfill_queued_once() -> Result<()> {
    let (storage, _dir) = setup().await?;
    legacy_tx_put(&storage, &folder("n1", "/n1")).await?;
    assert!(node_path_entries(&storage, "n1").is_empty());

    let pending = pending_node_path_branches(&storage)?;
    assert!(pending.iter().any(is_main), "{pending:?}");
    assert_eq!(enqueue_pending_node_path_backfills(&storage).await?, 1);
    let jobs = queued(&storage).await;
    assert_eq!(jobs.len(), 1, "one job for the branch");
    assert_eq!(jobs[0].0, BRANCH);

    // A second start while that job is still queued queues nothing.
    assert_eq!(enqueue_pending_node_path_backfills(&storage).await?, 0);

    // The job's work (the handler calls exactly this, with default options).
    let reports = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::NodePath,
        backfill_options(),
    )
    .await?;
    assert!(reports[0].completed);
    assert_eq!(node_path_entries(&storage, "n1").len(), 1);

    // Done on this node: no longer pending, so no later start queues it.
    let pending = pending_node_path_branches(&storage)?;
    assert!(!pending.iter().any(is_main), "{pending:?}");
    Ok(())
}

/// The review finding: on a first boot EVERY branch is pending, and one job
/// per branch queued at once filled the background pool with full-branch
/// scans while live writes' index jobs waited. Now one job is queued, and each
/// finished link queues the next pending branch.
#[tokio::test]
async fn first_boot_queues_one_backfill_and_chains_branch_by_branch() -> Result<()> {
    let (storage, _dir) = setup().await?;
    for fork in ["a-fork", "b-fork"] {
        storage
            .branches()
            .create_branch(TENANT, REPO, fork, "system", None, None, false, false)
            .await?;
    }
    let pending: Vec<String> = pending_node_path_branches(&storage)?
        .into_iter()
        .filter(|(t, r, _)| t == TENANT && r == REPO)
        .map(|(_, _, b)| b)
        .collect();
    assert_eq!(pending, ["a-fork", "b-fork", BRANCH]);

    // The start queues ONE job, for the first pending branch.
    assert_eq!(enqueue_pending_node_path_backfills(&storage).await?, 1);
    let jobs = queued(&storage).await;
    assert_eq!(
        jobs.iter().map(|j| j.0.as_str()).collect::<Vec<_>>(),
        ["a-fork"]
    );
    // ...and a second start, while that link is live, queues nothing.
    assert_eq!(enqueue_pending_node_path_backfills(&storage).await?, 0);

    // Each finished link queues the next branch after it — and only that one.
    let first = jobs[0].1.clone();
    let chained = continue_node_path_backfill_chain(&storage, &first, TENANT, REPO, Some("a-fork"));
    assert_eq!(chained.await?, 1);
    let jobs = queued(&storage).await;
    assert_eq!(
        jobs.iter().map(|j| j.0.as_str()).collect::<Vec<_>>(),
        ["a-fork", "b-fork"]
    );
    let second = jobs[1].1.clone();
    let chained =
        continue_node_path_backfill_chain(&storage, &second, TENANT, REPO, Some("b-fork"));
    assert_eq!(chained.await?, 1);
    assert_eq!(queued(&storage).await.len(), 3);
    let last = queued(&storage).await[2].1.clone();
    let chained = continue_node_path_backfill_chain(&storage, &last, TENANT, REPO, Some(BRANCH));
    assert_eq!(chained.await?, 0, "the chain ends after the last branch");

    // An admin-triggered run (no chain marker) continues nothing.
    let mut admin = first.clone();
    admin.metadata.clear();
    let chained = continue_node_path_backfill_chain(&storage, &admin, TENANT, REPO, None);
    assert_eq!(chained.await?, 0);
    Ok(())
}
