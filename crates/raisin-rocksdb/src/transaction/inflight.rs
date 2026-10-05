//! Revisions allocated to transactions that have not committed yet.
//!
//! A transaction allocates its revision at its FIRST write
//! (`get_or_allocate_transaction_revision`) and commits whenever its caller
//! says so — a `psql` session can sit inside `BEGIN` for as long as it likes,
//! and nothing checks conflicts at commit. Its entries then land at that old
//! revision, below everything committed in between. Run-collapse decides from
//! the entries it sees below its watermark (plan Phase 9): a late commit
//! landing between two versions collapse already merged would change the
//! answer at HEAD (a staged `T(ref)@r3` surfacing once `ref@r5` was collapsed
//! into `ref@r1`).
//!
//! So every allocated, uncommitted transaction revision is registered here,
//! per database, and [`oldest_inflight_revision`] clamps the watermark below
//! the oldest. The token deregisters on commit, rollback or drop.
//!
//! Process-local, like the database: one process owns it.

use raisin_hlc::HLC;
use rocksdb::DB;
use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex};

/// `db path -> revision -> open transactions holding it`.
static INFLIGHT: LazyLock<Mutex<HashMap<String, BTreeMap<HLC, usize>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// One open transaction's claim on its revision; dropping it releases it.
#[derive(Debug)]
pub(crate) struct InflightRevision {
    path: String,
    revision: HLC,
}

impl InflightRevision {
    /// Register `revision` as allocated and not yet committed on `db`.
    pub(crate) fn register(db: &DB, revision: HLC) -> Self {
        let path = db.path().to_string_lossy().into_owned();
        let mut map = INFLIGHT.lock().unwrap_or_else(|p| p.into_inner());
        *map.entry(path.clone())
            .or_default()
            .entry(revision)
            .or_default() += 1;
        Self { path, revision }
    }
}

impl Drop for InflightRevision {
    fn drop(&mut self) {
        let mut map = INFLIGHT.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(revs) = map.get_mut(&self.path) {
            if let Some(n) = revs.get_mut(&self.revision) {
                *n -= 1;
                if *n == 0 {
                    revs.remove(&self.revision);
                }
            }
            if revs.is_empty() {
                map.remove(&self.path);
            }
        }
    }
}

/// The oldest revision an open transaction on `db` may still commit at.
pub(crate) fn oldest_inflight_revision(db: &DB) -> Option<HLC> {
    let path = db.path().to_string_lossy();
    let map = INFLIGHT.lock().unwrap_or_else(|p| p.into_inner());
    map.get(path.as_ref())
        .and_then(|revs| revs.keys().next().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_open_revision_is_reported_until_released() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, dir.path()).unwrap();
        assert_eq!(oldest_inflight_revision(&db), None);
        let a = InflightRevision::register(&db, HLC::new(5, 0));
        let b = InflightRevision::register(&db, HLC::new(3, 0));
        let b2 = InflightRevision::register(&db, HLC::new(3, 0));
        assert_eq!(oldest_inflight_revision(&db), Some(HLC::new(3, 0)));
        drop(b);
        assert_eq!(oldest_inflight_revision(&db), Some(HLC::new(3, 0)));
        drop(b2);
        assert_eq!(oldest_inflight_revision(&db), Some(HLC::new(5, 0)));
        drop(a);
        assert_eq!(oldest_inflight_revision(&db), None);
    }
}
