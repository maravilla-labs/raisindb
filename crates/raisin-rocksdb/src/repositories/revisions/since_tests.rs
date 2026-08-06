//! Tests for [`RevisionRepository::list_revisions_since`] — the seek-based
//! watermark walk.
//!
//! Everything here is about the two properties the ordinary `list_revisions`
//! does NOT have: oldest-first ordering, and resuming from a point without
//! rescanning (or re-emitting) what came before.

use std::sync::Arc;

use chrono::Utc;
use raisin_hlc::HLC;
use raisin_storage::{RevisionMeta, RevisionRepository};

use super::RevisionRepositoryImpl;

const TENANT: &str = "t1";
const REPO: &str = "r1";

fn env() -> (tempfile::TempDir, Arc<crate::RocksDBStorage>) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(crate::RocksDBStorage::new(dir.path()).unwrap());
    (dir, storage)
}

fn repo(storage: &Arc<crate::RocksDBStorage>) -> &RevisionRepositoryImpl {
    use raisin_storage::Storage;
    storage.revisions()
}

fn meta(rev: HLC, actor: &str, branch: &str) -> RevisionMeta {
    RevisionMeta {
        revision: rev,
        parent: None,
        merge_parent: None,
        branch: branch.to_string(),
        timestamp: Utc::now(),
        actor: actor.to_string(),
        message: String::new(),
        is_system: false,
        changed_nodes: Vec::new(),
        changed_node_types: Vec::new(),
        changed_archetypes: Vec::new(),
        changed_element_types: Vec::new(),
        operation: None,
    }
}

/// Store revisions at `1..=n` milliseconds and hand back their HLCs.
async fn seed(storage: &Arc<crate::RocksDBStorage>, n: u64) -> Vec<HLC> {
    let mut out = Vec::new();
    for i in 1..=n {
        let rev = HLC::new(i, 0);
        repo(storage)
            .store_revision_meta(TENANT, REPO, meta(rev, "alice", "main"))
            .await
            .unwrap();
        out.push(rev);
    }
    out
}

/// The ordering contract: oldest first, the opposite of `list_revisions`.
///
/// A watermark walk that consumed newest-first would store, as "processed up
/// to", the newest revision it saw on its first page — skipping everything
/// older forever.
#[tokio::test]
async fn walks_forward_in_commit_order() {
    let (_dir, storage) = env();
    let revs = seed(&storage, 5).await;

    let got = repo(&storage)
        .list_revisions_since(TENANT, REPO, &HLC::new(0, 0), 10)
        .await
        .unwrap();

    assert_eq!(
        got.iter().map(|m| m.revision).collect::<Vec<_>>(),
        revs,
        "list_revisions_since must return oldest first"
    );
    // And the counterpart really is the other way round, so the two are not
    // interchangeable.
    let newest_first = repo(&storage)
        .list_revisions(TENANT, REPO, 10, 0)
        .await
        .unwrap();
    assert_eq!(newest_first[0].revision, revs[4]);
}

/// `after` is exclusive: feeding a watermark straight back must not re-deliver
/// the revision it names.
#[tokio::test]
async fn the_watermark_itself_is_never_redelivered() {
    let (_dir, storage) = env();
    let revs = seed(&storage, 5).await;

    let got = repo(&storage)
        .list_revisions_since(TENANT, REPO, &revs[2], 10)
        .await
        .unwrap();

    assert_eq!(
        got.iter().map(|m| m.revision).collect::<Vec<_>>(),
        vec![revs[3], revs[4]]
    );
}

/// Paging: each page resumes exactly where the last one stopped, with no gap
/// and no repeat.
///
/// The iteration cap is not defensive clutter — it is the assertion. An
/// inclusive bound makes this walk NON-TERMINATING (every page re-delivers the
/// watermark, so the watermark never advances), and a test that hangs reports
/// nothing at all: it burns a CI worker at 100% until something reaps it, with
/// no failure message anywhere.
#[tokio::test]
async fn pages_resume_without_gap_or_overlap() {
    let (_dir, storage) = env();
    let revs = seed(&storage, 7).await;

    let mut seen = Vec::new();
    let mut watermark = HLC::new(0, 0);
    for _ in 0..(revs.len() + 2) {
        let page = repo(&storage)
            .list_revisions_since(TENANT, REPO, &watermark, 3)
            .await
            .unwrap();
        if page.is_empty() {
            break;
        }
        watermark = page.last().unwrap().revision;
        seen.extend(page.into_iter().map(|m| m.revision));
        assert!(
            seen.len() <= revs.len(),
            "the walk is not making progress — a page re-delivered its own watermark"
        );
    }
    assert_eq!(seen, revs);
}

/// A watermark that no longer exists (GC'd, or simply a timestamp nobody
/// committed at) must resume at the next revision, not return nothing.
///
/// This is the reverse-seek detail: `Direction::Reverse` from an absent key
/// lands on the largest key `<=` it, which under the descending revision
/// encoding is the smallest revision `>=` the watermark.
#[tokio::test]
async fn a_watermark_that_was_never_committed_still_resumes() {
    let (_dir, storage) = env();
    let revs = seed(&storage, 5).await;

    // Between revs[1] (ms 2) and revs[2] (ms 3).
    let phantom = HLC::new(2, 500);
    let got = repo(&storage)
        .list_revisions_since(TENANT, REPO, &phantom, 10)
        .await
        .unwrap();

    assert_eq!(
        got.iter().map(|m| m.revision).collect::<Vec<_>>(),
        vec![revs[2], revs[3], revs[4]]
    );
}

/// The walk must stop at the repository's key range. Walking backwards leaves
/// the range at the NEWEST revision and lands in whatever precedes it in the
/// column family — another repo's history, in a multi-tenant store.
#[tokio::test]
async fn never_walks_out_of_the_repository() {
    let (_dir, storage) = env();
    seed(&storage, 3).await;
    for i in 1..=3u64 {
        repo(&storage)
            .store_revision_meta(
                "another-tenant",
                REPO,
                meta(HLC::new(i, 0), "mallory", "main"),
            )
            .await
            .unwrap();
        repo(&storage)
            .store_revision_meta(
                TENANT,
                "another-repo",
                meta(HLC::new(i, 0), "mallory", "main"),
            )
            .await
            .unwrap();
    }

    let got = repo(&storage)
        .list_revisions_since(TENANT, REPO, &HLC::new(0, 0), 100)
        .await
        .unwrap();

    assert_eq!(got.len(), 3);
    assert!(
        got.iter().all(|m| m.actor == "alice"),
        "leaked a revision from another tenant/repo: {:?}",
        got.iter().map(|m| m.actor.clone()).collect::<Vec<_>>()
    );
}

/// Same-millisecond commits are ordered by the HLC counter, not collapsed.
#[tokio::test]
async fn orders_by_counter_within_one_millisecond() {
    let (_dir, storage) = env();
    for c in 0..4u64 {
        repo(&storage)
            .store_revision_meta(TENANT, REPO, meta(HLC::new(9, c), "alice", "main"))
            .await
            .unwrap();
    }

    let got = repo(&storage)
        .list_revisions_since(TENANT, REPO, &HLC::new(9, 1), 10)
        .await
        .unwrap();

    assert_eq!(
        got.iter().map(|m| m.revision).collect::<Vec<_>>(),
        vec![HLC::new(9, 2), HLC::new(9, 3)]
    );
}

/// A zero limit asks for nothing and must not be read as "unbounded".
#[tokio::test]
async fn a_zero_limit_returns_nothing() {
    let (_dir, storage) = env();
    seed(&storage, 3).await;
    assert!(repo(&storage)
        .list_revisions_since(TENANT, REPO, &HLC::new(0, 0), 0)
        .await
        .unwrap()
        .is_empty());
}
