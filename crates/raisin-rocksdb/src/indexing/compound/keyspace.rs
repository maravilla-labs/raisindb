//! One compound index's KEYSPACE as a unit: the process-wide build lock every
//! builder takes, and the clear a build (or a drop) runs.
//!
//! The build lease the job handler takes is scoped to this node and absent
//! without a lock manager, and two builds of one index interleaving — one
//! clearing the keyspace after the other wrote part of it — would leave a
//! keyspace mixing two builds' floors. (The build ticket of
//! `CompoundStateStore::begin_rebuild` lets only the LAST registration stamp
//! `Ready`, but the earlier builder may still be writing into the later one's
//! keyspace.) Every builder of a keyspace — the per-index job, the automatic
//! `compound_builds` chain (plan Phase 13f), the admin `REBUILD … compound`
//! (every index it rebuilds, in name order), the drop of an undeclared
//! workspace index — takes [`lock`] first, so builds of one keyspace QUEUE in
//! this process (the
//! compound keyspace is local to the node, so the process is the whole
//! scope). A waiter re-checks the state once it holds the lock: the build it
//! waited for usually made its own redundant.

use crate::jobs::{KeyedMutex, KeyedMutexGuard};
use crate::{cf, cf_handle, keys};
use raisin_error::{Error, Result};
use rocksdb::{WriteBatch, DB};
use std::sync::{Arc, OnceLock};

/// `{database path, tenant, repo, branch, workspace, index}`.
type Key = (String, String, String, String, String, String);

fn locks() -> &'static Arc<KeyedMutex<Key>> {
    static LOCKS: OnceLock<Arc<KeyedMutex<Key>>> = OnceLock::new();
    LOCKS.get_or_init(|| Arc::new(KeyedMutex::new()))
}

/// Hold the keyspace of one index exclusively in this process, waiting for
/// any build or drop of it already running.
pub async fn lock(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    index_name: &str,
) -> KeyedMutexGuard<Key> {
    locks()
        .lock((
            db.path().to_string_lossy().into_owned(),
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            workspace.to_string(),
            index_name.to_string(),
        ))
        .await
}

/// Delete every entry of one index (both publication tags), in bounded
/// batches. The caller holds [`lock`].
pub fn clear(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    index_name: &str,
) -> Result<u64> {
    let cf_compound = cf_handle(db, cf::COMPOUND_INDEX)?;
    let mut deleted = 0u64;
    for published in [false, true] {
        let prefix = keys::compound_index_prefix(
            tenant_id,
            repo_id,
            branch,
            workspace,
            index_name,
            &[],
            published,
        );
        let mut batch = WriteBatch::default();
        for item in crate::prefix_scan(db, cf_compound, &prefix) {
            let (key, _) = item.map_err(|e| Error::storage(e.to_string()))?;
            if !key.starts_with(&prefix) {
                break;
            }
            batch.delete_cf(cf_compound, key);
            deleted += 1;
            if batch.len() >= 10_000 {
                db.write(std::mem::take(&mut batch))
                    .map_err(|e| Error::storage(e.to_string()))?;
            }
        }
        db.write(batch).map_err(|e| Error::storage(e.to_string()))?;
    }
    Ok(deleted)
}

/// How many entries (live and tombstones, both tags) one index holds — for
/// tests and the drop's report.
pub fn count(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    index_name: &str,
) -> Result<u64> {
    let cf_compound = cf_handle(db, cf::COMPOUND_INDEX)?;
    let mut n = 0u64;
    for published in [false, true] {
        let prefix = keys::compound_index_prefix(
            tenant_id,
            repo_id,
            branch,
            workspace,
            index_name,
            &[],
            published,
        );
        for item in crate::prefix_scan(db, cf_compound, &prefix) {
            let (key, _) = item.map_err(|e| Error::storage(e.to_string()))?;
            if !key.starts_with(&prefix) {
                break;
            }
            n += 1;
        }
    }
    Ok(n)
}
