//! Per-node commit serialization (plan Phase 7b).
//!
//! Semantics stay last-writer-wins: transactions are not isolated and nothing
//! is rejected or retried. What this lock fixes is narrower — the derived
//! indexes of ONE node disagreeing with that node's records. A write proves
//! its index delta against the versions stored when it is STAGED; two
//! overlapping writers of one node can then commit out of revision order, and
//! the delta (`index.skip_unchanged` above all) leaves out an entry the
//! winning version needs. The commit step therefore re-validates the staged
//! delta against what is stored NOW (`StagedDeltaCheck::revalidate_final`) and
//! writes, and this lock makes "now" mean something: no other writer of the
//! node can land between the re-validation and the write.
//!
//! - **In-process is enough, and cluster-safe.** Derived indexes are
//!   node-local — nothing indexed replicates (CLAUDE.md) — so only writers in
//!   THIS process can race on this node's index. No `raisin_locks`, no Redis,
//!   no `[locks]` section, no config.
//! - **Held only around the commit step**: re-validate, write, release. Never
//!   across staging, validation or event emission.
//! - **Every funnel takes it**: the transaction commit, the repository
//!   writers (update, add, delete, cascade, move, reorder, rebalance, copy,
//!   deep create, cross-branch stage), merge apply, and the replication
//!   applicator for each node an op touches.
//! - **Lock order**: a commit locks all its nodes at once, in sorted key
//!   order ([`lock_nodes`]), and BEFORE the branch record lock
//!   (`repositories::lock_branch_record`). Never lock nodes while holding the
//!   branch record lock, and never lock nodes twice in one call chain (the
//!   mutex is not re-entrant).
//! - **Async only.** A waiter queues on a tokio mutex, never blocking a
//!   runtime thread; the write it guards runs on the blocking pool.
//! - **The guard travels INTO the blocking write task** and drops only after
//!   `db.write` returns. Held in the async frame instead, a commit future
//!   dropped while it awaits the write (a client disconnect, a timeout
//!   wrapper) released the lock with the batch still in flight, and another
//!   writer of the node re-validated against stored state that lacked it.
//!
//! Keys carry the database's identity, so two storages in one process (a test
//! origin and its replica) never contend.

use crate::jobs::keyed_mutex::{KeyedMutex, KeyedMutexGuard};
use rocksdb::DB;
use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};

/// One node on one branch of one database. The branch scope is shared by
/// every key of one commit (one allocation, not three per node).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeLockKey {
    db: usize,
    /// `(tenant, repo, branch)`
    scope: Arc<(String, String, String)>,
    node_id: String,
}

static LOCKS: LazyLock<Arc<KeyedMutex<NodeLockKey>>> =
    LazyLock::new(|| Arc::new(KeyedMutex::new()));

/// The identity a lock key carries for `db`.
pub(crate) fn db_identity(db: &DB) -> usize {
    db as *const DB as usize
}

/// Exclusive hold on a set of nodes; released (in reverse order) on drop.
pub struct NodeWriteGuard {
    held: Vec<KeyedMutexGuard<NodeLockKey>>,
    db: usize,
    node_ids: Vec<String>,
}

impl NodeWriteGuard {
    /// How many nodes this guard holds.
    pub fn len(&self) -> usize {
        self.held.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// The point between a commit's re-validation and its write. A no-op
    /// (one atomic load) unless a test registered a pause for one of these
    /// nodes (`test_hooks::pause_commit_of`).
    pub async fn before_write(&self) {
        test_hooks::pause_point(self.db, &self.node_ids).await;
    }

    /// Inside the blocking write task, right before `db.write`. A no-op (one
    /// atomic load) unless a test registered a write pause for one of these
    /// nodes (`test_hooks::pause_write_of`).
    pub fn in_write(&self) {
        test_hooks::write_pause_point(self.db, &self.node_ids);
    }
}

impl Drop for NodeWriteGuard {
    fn drop(&mut self) {
        while let Some(guard) = self.held.pop() {
            drop(guard);
        }
    }
}

/// Lock `node_ids` on `(tenant, repo, branch)` of `db`, in sorted order
/// (duplicates collapse), waiting for any current holder of each.
pub async fn lock_nodes<'a, I>(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_ids: I,
) -> NodeWriteGuard
where
    I: IntoIterator<Item = &'a str>,
{
    let db_id = db_identity(db);
    let scope = Arc::new((
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
    ));
    let keys: BTreeSet<NodeLockKey> = node_ids
        .into_iter()
        .map(|node_id| NodeLockKey {
            db: db_id,
            scope: scope.clone(),
            node_id: node_id.to_string(),
        })
        .collect();
    let node_ids: Vec<String> = keys.iter().map(|k| k.node_id.clone()).collect();
    let mut held = Vec::with_capacity(keys.len());
    if !test_hooks::locks_disabled(db_id) {
        for key in keys {
            held.push(LOCKS.lock(key).await);
        }
    }
    NodeWriteGuard {
        held,
        db: db_id,
        node_ids,
    }
}

/// Node keys currently locked or waited on, process-wide (monitoring, tests).
pub fn nodes_in_flight() -> usize {
    LOCKS.len()
}

#[doc(hidden)]
pub mod test_hooks;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn db() -> (tempfile::TempDir, DB) {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, dir.path()).unwrap();
        (dir, db)
    }

    #[tokio::test]
    async fn opposite_orders_do_not_deadlock() {
        let (_dir, db) = db();
        let db = Arc::new(db);
        let mut tasks = Vec::new();
        for i in 0..64 {
            let db = db.clone();
            tasks.push(tokio::spawn(async move {
                let ids: Vec<&str> = if i % 2 == 0 {
                    vec!["a", "b", "c"]
                } else {
                    vec!["c", "b", "a"]
                };
                let guard = lock_nodes(&db, "t", "r", "main", ids).await;
                assert_eq!(guard.len(), 3);
                tokio::task::yield_now().await;
            }));
        }
        for task in tasks {
            tokio::time::timeout(Duration::from_secs(10), task)
                .await
                .expect("no deadlock")
                .unwrap();
        }
    }

    #[tokio::test]
    async fn same_node_on_another_database_does_not_contend() {
        let (_d1, db1) = db();
        let (_d2, db2) = db();
        let _held = lock_nodes(&db1, "t", "r", "main", ["n"]).await;
        tokio::time::timeout(
            Duration::from_millis(500),
            lock_nodes(&db2, "t", "r", "main", ["n"]),
        )
        .await
        .expect("a different database must not block");
    }
}
