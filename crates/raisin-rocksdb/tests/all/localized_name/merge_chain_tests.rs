//! Regressions from the Phase 12 review: what a merge owes the target's
//! index, and the automatic build chain reaching every branch.

use super::support::*;
use raisin_context::MergeStrategy;
use raisin_rocksdb::cf;
use raisin_rocksdb::localized_name::Availability;
use raisin_rocksdb::management::async_indexing::repair::{
    after_link, enqueue_branch, start_chain, RepairKind,
};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::jobs::{JobContext, JobType};
use raisin_storage::localized::LocalizedServedBy::Index;
use raisin_storage::{BranchRepository, Storage};
use std::sync::Arc;

const DRAFT: &str = "draft";

async fn fork(storage: &RocksDBStorage, name: &str) {
    storage
        .branches()
        .create_branch(T, R, name, "test", None, Some(B.to_string()), false, false)
        .await
        .unwrap();
}

/// On `draft`: `/products/table`, named `tableau` in `fr`.
async fn table_on_draft(storage: &Arc<RocksDBStorage>) -> String {
    let table = create_on(storage, DRAFT, page("/products/table", &[])).await;
    let data = std::collections::HashMap::from([(
        raisin_models::translations::JsonPointer::new("/__node_name"),
        raisin_models::nodes::properties::PropertyValue::String("tableau".into()),
    )]);
    translations(storage)
        .update_translation(T, R, DRAFT, WS, &table, &code("fr"), data, "test", None)
        .await
        .unwrap();
    table
}

async fn merge_draft(storage: &RocksDBStorage) {
    let result = storage
        .branches_impl()
        .merge_branches(T, R, B, DRAFT, MergeStrategy::ThreeWay, "merge", "test")
        .await
        .unwrap();
    assert!(result.success, "{result:?}");
}

/// Delete every index row of `branch` (rows written before the index
/// existed, or while it was switched off, look exactly like this).
fn wipe_rows(storage: &RocksDBStorage, branch: &str) {
    let db = storage.db();
    let cf = db.cf_handle(cf::LOCALIZED_NAME_INDEX).unwrap();
    let from = format!("{T}\0{R}\0{branch}\0").into_bytes();
    let mut to = from.clone();
    *to.last_mut().unwrap() = 1;
    db.delete_range_cf(cf, &from, &to).unwrap();
}

/// The source's rows lack a node the merge brings in: the copy replayed no
/// claim, the target stayed `Ready`, and its localized URL 404'd for good.
#[tokio::test]
async fn merge_restores_names_the_source_rows_lack() {
    let (storage, _dir) = open().await;
    catalog(&storage).await;
    build(&storage, B).await;
    fork(&storage, DRAFT).await;
    build(&storage, DRAFT).await;
    let table = table_on_draft(&storage).await;
    wipe_rows(&storage, DRAFT);

    merge_draft(&storage).await;
    assert!(availability(&storage, B).is_ready());
    assert_eq!(
        id_via(&storage, B, "fr", "/produits/tableau", Index),
        Some(table)
    );
}

/// A merge from a source whose index is not built leaves the target's
/// `Ready` claiming a completeness the copied rows do not have.
#[tokio::test]
async fn merge_from_unbuilt_source_flips_target() {
    let (storage, _dir) = open().await;
    catalog(&storage).await;
    build(&storage, B).await;
    fork(&storage, DRAFT).await;
    assert_eq!(availability(&storage, DRAFT), Availability::NotBuilt);
    let table = table_on_draft(&storage).await;

    merge_draft(&storage).await;
    assert_eq!(availability(&storage, B), Availability::NotBuilt);
    build(&storage, B).await;
    assert_eq!(
        id_via(&storage, B, "fr", "/produits/tableau", Index),
        Some(table)
    );
}

/// The queued `localized_names` jobs, by branch, with their contexts.
async fn queued(storage: &RocksDBStorage) -> Vec<(String, JobContext)> {
    let mut out: Vec<(String, JobContext)> = storage
        .job_registry()
        .list_jobs_by_tenant(T)
        .await
        .into_iter()
        .filter_map(|job| match &job.job_type {
            JobType::IndexRepair {
                branch: Some(branch),
                repair,
                dry_run: false,
                ..
            } if repair == RepairKind::LocalizedNames.slug() => {
                let context = storage.job_data_store().get(T, &job.id).unwrap()?;
                Some((branch.clone(), context))
            }
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Only `node_path` links continued the chain: one start built ONE branch,
/// and every other branch stayed on the O(children) fallback.
#[tokio::test]
async fn localized_names_chain_continues_to_every_pending_branch() {
    let (storage, _dir) = open().await;
    fork(&storage, "a-fork").await;
    fork(&storage, "b-fork").await;
    let kind = RepairKind::LocalizedNames;

    assert_eq!(start_chain(&storage, kind).await.unwrap(), 1);
    let jobs = queued(&storage).await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].0, "a-fork");

    // A requested branch is queued directly, not after the chain.
    assert_eq!(enqueue_branch(&storage, kind, T, R, B).await.unwrap(), 1);
    assert_eq!(enqueue_branch(&storage, kind, T, R, B).await.unwrap(), 0);

    // The first link ends (here: failed); the chain moves on all the same.
    let first = jobs[0].1.clone();
    after_link(&storage, kind, &first, (T, R, Some("a-fork")), false).await;
    let branches: Vec<String> = queued(&storage).await.into_iter().map(|j| j.0).collect();
    assert_eq!(branches, vec!["a-fork", "b-fork", "main"]);
}
