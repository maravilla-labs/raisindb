//! A merge into a branch, from its first write until the source's entries are
//! replayed into the target (`copy_branch_indexes`), turns skip-unchanged OFF
//! on that branch.
//!
//! Merge replays the source branch's index entries at their ORIGINAL
//! revisions, after moving HEAD and outside the merge batch. A local write to
//! a merged node in that window, skipping an unchanged entry, keeps it at the
//! base revision — and a replayed source tombstone at `base < s1 < R` then
//! masks it. With the window open the write is a full put at R, which no
//! tombstone below R can mask.
//!
//! Process-local (merges run on the node that serves them); keyed by the
//! database instance as well, so two storages in one process (tests) never
//! see each other's merges.

use rocksdb::DB;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

type WindowKey = (usize, String, String, String);

static OPEN: LazyLock<Mutex<HashMap<WindowKey, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn key(db: &Arc<DB>, tenant_id: &str, repo_id: &str, branch: &str) -> WindowKey {
    (
        Arc::as_ptr(db) as usize,
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    )
}

/// Held for the duration of a merge into `branch`; dropping it closes the
/// window.
pub(crate) struct MergeWindow(WindowKey);

impl Drop for MergeWindow {
    fn drop(&mut self) {
        let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = open.get_mut(&self.0) {
            *count -= 1;
            if *count == 0 {
                open.remove(&self.0);
            }
        }
    }
}

/// Open the merge window on `branch` (nested merges count).
pub(crate) fn open_merge_window(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> MergeWindow {
    let key = key(db, tenant_id, repo_id, branch);
    *OPEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(key.clone())
        .or_insert(0) += 1;
    MergeWindow(key)
}

/// Whether a merge into `branch` is in progress.
pub(crate) fn merge_window_open(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> bool {
    OPEN.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(&key(db, tenant_id, repo_id, branch))
}
