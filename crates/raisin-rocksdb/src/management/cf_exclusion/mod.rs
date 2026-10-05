//! Per-`(branch, column family)` exclusion between run-collapse GC and the
//! writers that insert index entries BELOW existing ones, plus a per-database
//! exclusion between run-collapse and retention GC (plan Phase 9).
//!
//! Run-collapse deletes a version whose next-older version has the same
//! state. That decision is only valid while nothing inserts between the two:
//! a repair writing a tombstone at a historical delete revision, a rebuild
//! clearing and re-deriving a keyspace, or a branch copy (fork or merge)
//! bringing a source's history over. Those are the **inserters**; they hold
//! the exclusion SHARED for as long as they write (several may run at once,
//! as before), and a collapse SLICE holds it EXCLUSIVELY for one bounded batch
//! — read, decide, delete — and releases it before it sleeps.
//!
//! **Retention GC** is the other deleter, and it deletes the OPPOSITE end of
//! the same pair: it keeps the newest version at or below its cutoff and
//! deletes the older ones, where collapse keeps the oldest of a run and
//! deletes the newer twin. Both deciding from their own snapshot and both
//! committing empties the group. So a retention run holds a per-DATABASE
//! **pruner** hold (it walks every branch of every CF through one iterator):
//! it waits out a collapse slice in progress, and no slice starts while it
//! runs — collapse reports `busy` instead.
//!
//! A collapse only ever `try`s: it never waits on an inserter or a pruner
//! (either can run for hours), so it reports `busy` and leaves a resumable
//! cursor. An inserter or pruner waits for at most one slice, and a waiting
//! one is preferred: no new slice starts while one waits, so a fast loop of
//! slices cannot starve it. Nothing waits while holding, so there is no lock
//! cycle.
//!
//! Every inserter and pruner acquisition bumps an **epoch**. A collapse keeps
//! its per-group "newer entry" memory across slices only while the epoch is
//! unchanged — after an inserter or pruner ran, an entry may now sit between
//! (or be missing from) the two versions it remembered, so it forgets them.
//!
//! Process-local: one process owns a database. The key carries the database
//! path, so two databases in one process (tests) never contend.

#[cfg(test)]
mod tests;

use rocksdb::DB;
use std::collections::HashMap;
use std::sync::{Condvar, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

/// `(db path, tenant, repo, branch, cf)`.
type Key = (String, String, String, String, String);

#[derive(Default)]
struct Slot {
    inserters: usize,
    /// Inserters parked until the current slice ends: no new slice starts.
    waiting: usize,
    collapsing: bool,
    epoch: u64,
}

/// Per-database state: retention GC runs and collapse slices in flight.
#[derive(Default)]
struct DbSlot {
    pruners: usize,
    waiting_pruners: usize,
    /// Collapse slices in progress on any `(branch, CF)` of this database.
    collapsing: usize,
    epoch: u64,
}

#[derive(Default)]
struct Registry {
    slots: HashMap<Key, Slot>,
    dbs: HashMap<String, DbSlot>,
}

static REGISTRY: LazyLock<(Mutex<Registry>, Condvar)> =
    LazyLock::new(|| (Mutex::new(Registry::default()), Condvar::new()));

fn lock() -> MutexGuard<'static, Registry> {
    REGISTRY.0.lock().unwrap_or_else(|p| p.into_inner())
}

fn db_path(db: &DB) -> String {
    db.path().to_string_lossy().into_owned()
}

fn key(db: &DB, tenant: &str, repo: &str, branch: &str, cf: &str) -> Key {
    (
        db_path(db),
        tenant.to_string(),
        repo.to_string(),
        branch.to_string(),
        cf.to_string(),
    )
}

/// A shared hold by a writer that inserts below existing entries.
pub struct InserterGuard {
    key: Key,
}

impl Drop for InserterGuard {
    fn drop(&mut self) {
        let mut reg = lock();
        if let Some(slot) = reg.slots.get_mut(&self.key) {
            slot.inserters = slot.inserters.saturating_sub(1);
        }
        REGISTRY.1.notify_all();
    }
}

/// A hold by one retention GC run over a whole database.
pub struct PrunerGuard {
    path: String,
}

impl Drop for PrunerGuard {
    fn drop(&mut self) {
        let mut reg = lock();
        if let Some(db) = reg.dbs.get_mut(&self.path) {
            db.pruners = db.pruners.saturating_sub(1);
        }
        REGISTRY.1.notify_all();
    }
}

/// An exclusive hold by one collapse slice.
pub struct CollapseGuard {
    key: Key,
    epoch: u64,
}

impl CollapseGuard {
    /// The inserter and pruner epoch when this slice started.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

impl Drop for CollapseGuard {
    fn drop(&mut self) {
        let mut reg = lock();
        if let Some(slot) = reg.slots.get_mut(&self.key) {
            slot.collapsing = false;
        }
        if let Some(db) = reg.dbs.get_mut(&self.key.0) {
            db.collapsing = db.collapsing.saturating_sub(1);
        }
        REGISTRY.1.notify_all();
    }
}

/// Counts a parked waiter for as long as it waits, and un-counts it however
/// the wait ends — including an async waiter whose future is dropped.
struct Waiting<F: Fn(&mut Registry)> {
    undo: Option<F>,
}

impl<F: Fn(&mut Registry)> Waiting<F> {
    fn finish(mut self, reg: &mut Registry) {
        if let Some(undo) = self.undo.take() {
            undo(reg);
        }
    }
}

impl<F: Fn(&mut Registry)> Drop for Waiting<F> {
    fn drop(&mut self) {
        if let Some(undo) = self.undo.take() {
            undo(&mut lock());
            REGISTRY.1.notify_all();
        }
    }
}

fn inserter_waiting(key: &Key) -> impl Fn(&mut Registry) + '_ {
    move |reg: &mut Registry| {
        if let Some(slot) = reg.slots.get_mut(key) {
            slot.waiting = slot.waiting.saturating_sub(1);
        }
    }
}

/// Take the shared hold if no slice is in progress (and count the caller as
/// waiting otherwise).
fn try_enter_inserter(reg: &mut Registry, key: &Key) -> bool {
    let slot = reg.slots.entry(key.clone()).or_default();
    if slot.collapsing {
        return false;
    }
    slot.inserters += 1;
    slot.epoch += 1;
    true
}

/// Take the shared hold, waiting out a collapse slice in progress. For
/// blocking contexts (repairs, branch copies run on blocking threads).
pub fn enter_inserter(db: &DB, tenant: &str, repo: &str, branch: &str, cf: &str) -> InserterGuard {
    let key = key(db, tenant, repo, branch, cf);
    let mut reg = lock();
    if try_enter_inserter(&mut reg, &key) {
        return InserterGuard { key };
    }
    reg.slots.entry(key.clone()).or_default().waiting += 1;
    let waiting = Waiting {
        undo: Some(inserter_waiting(&key)),
    };
    loop {
        reg = REGISTRY.1.wait(reg).unwrap_or_else(|p| p.into_inner());
        if try_enter_inserter(&mut reg, &key) {
            waiting.finish(&mut reg);
            return InserterGuard { key };
        }
    }
}

/// [`enter_inserter`] for async contexts: polls instead of parking a worker.
pub async fn enter_inserter_async(
    db: &DB,
    tenant: &str,
    repo: &str,
    branch: &str,
    cf: &str,
) -> InserterGuard {
    let key = key(db, tenant, repo, branch, cf);
    {
        let mut reg = lock();
        if try_enter_inserter(&mut reg, &key) {
            return InserterGuard { key };
        }
        reg.slots.entry(key.clone()).or_default().waiting += 1;
    }
    let waiting = Waiting {
        undo: Some(inserter_waiting(&key)),
    };
    loop {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let mut reg = lock();
        if try_enter_inserter(&mut reg, &key) {
            waiting.finish(&mut reg);
            return InserterGuard { key };
        }
    }
}

/// Take the per-database hold for one retention GC run, waiting out the
/// collapse slices in progress. Blocking: call it from a blocking context.
pub fn enter_pruner(db: &DB) -> PrunerGuard {
    let path = db_path(db);
    let mut reg = lock();
    reg.dbs.entry(path.clone()).or_default().waiting_pruners += 1;
    let undo = |reg: &mut Registry| {
        if let Some(d) = reg.dbs.get_mut(&path) {
            d.waiting_pruners = d.waiting_pruners.saturating_sub(1);
        }
    };
    let waiting = Waiting { undo: Some(undo) };
    loop {
        let slot = reg.dbs.entry(path.clone()).or_default();
        if slot.collapsing == 0 {
            slot.pruners += 1;
            slot.epoch += 1;
            waiting.finish(&mut reg);
            return PrunerGuard { path };
        }
        reg = REGISTRY.1.wait(reg).unwrap_or_else(|p| p.into_inner());
    }
}

/// Try to take the exclusive hold for one collapse slice; `None` while any
/// inserter holds or waits for it, another collapse holds it, or a retention
/// GC run holds or waits for the database.
pub fn try_collapse(
    db: &DB,
    tenant: &str,
    repo: &str,
    branch: &str,
    cf: &str,
) -> Option<CollapseGuard> {
    let key = key(db, tenant, repo, branch, cf);
    let mut reg = lock();
    let db_slot = reg.dbs.entry(key.0.clone()).or_default();
    if db_slot.pruners > 0 || db_slot.waiting_pruners > 0 {
        return None;
    }
    let db_epoch = db_slot.epoch;
    let slot = reg.slots.entry(key.clone()).or_default();
    if slot.collapsing || slot.inserters > 0 || slot.waiting > 0 {
        return None;
    }
    slot.collapsing = true;
    // Both epochs only grow, so their sum changes whenever either does.
    let epoch = slot.epoch + db_epoch;
    reg.dbs.entry(key.0.clone()).or_default().collapsing += 1;
    Some(CollapseGuard { key, epoch })
}

/// [`try_collapse`], retried for up to `patience` (an inserter holding only
/// briefly — a branch copy of a small CF — should not fail a whole run).
pub fn collapse_within(
    db: &DB,
    tenant: &str,
    repo: &str,
    branch: &str,
    cf: &str,
    patience: Duration,
) -> Option<CollapseGuard> {
    let started = std::time::Instant::now();
    loop {
        if let Some(guard) = try_collapse(db, tenant, repo, branch, cf) {
            return Some(guard);
        }
        if started.elapsed() >= patience {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
