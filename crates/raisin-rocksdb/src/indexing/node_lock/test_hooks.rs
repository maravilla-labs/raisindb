//! Test hooks: deterministic interleavings, and proof that a test fails
//! without the lock. Every hook is scoped to ONE database, so tests running
//! in parallel in one binary never see each other's hooks.

use super::db_identity;
use rocksdb::DB;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::Notify;

static ACTIVE: AtomicBool = AtomicBool::new(false);
/// One-shot pauses keyed by `(database identity, node id)`.
type Registry<T> = LazyLock<Mutex<HashMap<(usize, String), Arc<T>>>>;

static PAUSES: Registry<CommitPause> = LazyLock::new(|| Mutex::new(HashMap::new()));
static DISABLED: LazyLock<Mutex<HashSet<usize>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
static WRITE_PAUSES: Registry<WritePause> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// A one-shot pause of the next commit of one node INSIDE its blocking
/// write task, right before `db.write`. Its methods block: call them from
/// a blocking context (`spawn_blocking`).
#[derive(Default)]
pub struct WritePause {
    /// `(reached, released)`
    state: Mutex<(bool, bool)>,
    changed: std::sync::Condvar,
}

impl WritePause {
    /// Block until the paused write is about to write, or `timeout`.
    /// Returns whether it was reached.
    pub fn wait_reached(&self, timeout: std::time::Duration) -> bool {
        let state = self.state.lock().expect("write pause");
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |s| !s.0)
            .expect("write pause");
        state.0
    }

    /// Let the paused write proceed.
    pub fn release(&self) {
        self.state.lock().expect("write pause").1 = true;
        self.changed.notify_all();
    }
}

/// Pause the next commit on `db` that writes `node_id`, inside its
/// blocking write task.
pub fn pause_write_of(db: &DB, node_id: &str) -> Arc<WritePause> {
    let pause = Arc::new(WritePause::default());
    WRITE_PAUSES
        .lock()
        .expect("write pause registry")
        .insert((db_identity(db), node_id.to_string()), pause.clone());
    ACTIVE.store(true, Ordering::SeqCst);
    pause
}

pub(super) fn write_pause_point(db: usize, node_ids: &[String]) {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let pause = {
        let mut pauses = WRITE_PAUSES.lock().expect("write pause registry");
        node_ids
            .iter()
            .find_map(|id| pauses.remove(&(db, id.clone())))
    };
    if let Some(pause) = pause {
        let mut state = pause.state.lock().expect("write pause");
        state.0 = true;
        pause.changed.notify_all();
        let _released = pause
            .changed
            .wait_while(state, |s| !s.1)
            .expect("write pause");
    }
}

/// A one-shot pause of the next commit of one node, between its
/// re-validation and its write.
#[derive(Default)]
pub struct CommitPause {
    reached: Notify,
    release: Notify,
}

impl CommitPause {
    /// Wait until the paused commit has re-validated and is waiting.
    pub async fn reached(&self) {
        self.reached.notified().await;
    }

    /// Let the paused commit write.
    pub fn release(&self) {
        self.release.notify_one();
    }
}

/// Pause the next commit on `db` that writes `node_id`.
pub fn pause_commit_of(db: &DB, node_id: &str) -> Arc<CommitPause> {
    let pause = Arc::new(CommitPause::default());
    PAUSES
        .lock()
        .expect("pause registry")
        .insert((db_identity(db), node_id.to_string()), pause.clone());
    ACTIVE.store(true, Ordering::SeqCst);
    pause
}

/// Take no node locks on `db` (proves a test fails without them).
pub fn disable_node_locks(db: &DB, disabled: bool) {
    let mut set = DISABLED.lock().expect("disabled registry");
    if disabled {
        set.insert(db_identity(db));
        ACTIVE.store(true, Ordering::SeqCst);
    } else {
        set.remove(&db_identity(db));
    }
}

pub(super) fn locks_disabled(db: usize) -> bool {
    ACTIVE.load(Ordering::Relaxed)
        && DISABLED
            .lock()
            .map(|set| set.contains(&db))
            .unwrap_or(false)
}

pub(super) async fn pause_point(db: usize, node_ids: &[String]) {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let pause = {
        let mut pauses = PAUSES.lock().expect("pause registry");
        node_ids
            .iter()
            .find_map(|id| pauses.remove(&(db, id.clone())))
    };
    if let Some(pause) = pause {
        pause.reached.notify_one();
        pause.release.notified().await;
    }
}
