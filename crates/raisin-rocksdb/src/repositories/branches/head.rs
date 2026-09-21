//! Branch HEAD management
//!
//! HEAD operations (get_head, update_head) are implemented as part of
//! the BranchRepository trait in crud.rs, as they are core branch operations.
//!
//! This module owns the lock that makes those operations safe to run
//! concurrently.
//!
//! # Why a lock
//!
//! Every writer of a branch record reads it, edits a field, and writes the
//! whole record back — usually inside the same `WriteBatch` as the nodes whose
//! revision the new HEAD names. The monotonic guard ("only advance") compares
//! against the record as READ, so without mutual exclusion two writers can
//! both read HEAD=`r0`, both pass the guard, and land in either order:
//!
//! ```text
//! A (rev r2) reads head=r0 ─┐           ┌─ A writes head=r2
//! B (rev r1) reads head=r0 ─┴─ both ok ─┴─ B writes head=r1   ← HEAD regressed
//! ```
//!
//! A's nodes are then durable but ABOVE HEAD, so every `at_revision(head)`
//! read — trigger matching, the fulltext indexer, the REST read path — reports
//! them missing until some later commit on the branch moves HEAD past them.
//! Measured live: an AI tool-result aggregation node created at `…972-1` was
//! hidden by a tool-call status update allocated `…972-0` that wrote its batch
//! a moment later, so the continuation trigger never matched and the agent
//! hung.
//!
//! Holding [`lock_branch_record`] from the read through the batch write makes
//! the read-check-write atomic with respect to every other writer of the same
//! branch record.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use tokio::sync::{Mutex, MutexGuard};

/// Striped so unrelated branches rarely contend, and bounded so the lock table
/// never grows with the number of branches ever seen.
const BRANCH_RECORD_LOCK_STRIPES: usize = 64;

/// Process-wide rather than per repository instance: several
/// `BranchRepositoryImpl` / transaction instances can write the same branch
/// record, and they must all agree on one lock.
static BRANCH_RECORD_LOCKS: LazyLock<Vec<Mutex<()>>> = LazyLock::new(|| {
    (0..BRANCH_RECORD_LOCK_STRIPES)
        .map(|_| Mutex::new(()))
        .collect()
});

/// Serialize a read-modify-write of one branch record.
///
/// Hold the guard from reading the branch record until the write carrying the
/// modified record has landed, then drop it. Never acquire a second branch
/// record lock while holding one: stripes are shared between branches, so
/// nesting could deadlock.
pub(crate) async fn lock_branch_record(
    tenant_id: &str,
    repo_id: &str,
    branch_name: &str,
) -> MutexGuard<'static, ()> {
    let mut hasher = DefaultHasher::new();
    (tenant_id, repo_id, branch_name).hash(&mut hasher);
    let stripe = (hasher.finish() as usize) % BRANCH_RECORD_LOCK_STRIPES;
    BRANCH_RECORD_LOCKS[stripe].lock().await
}
