//! Serializes record writes at an EXISTING revision against the `node_path`
//! backfill.
//!
//! A record write normally lands at a freshly allocated revision, so nothing
//! else writes at it. Three writers instead rewrite a node IN PLACE, at the
//! revision its current version is already stored at:
//!
//! - the transaction `put_node` of a `versionable=false` node,
//! - the repository's `update_impl` of such a node,
//! - the replicated upsert at a revision a version is already stored at.
//!
//! The `node_path` backfill writes at an existing revision too: `NODE_PATH`
//! at R for a legacy full-`Node` blob at R. It checks that the blob is still
//! that legacy blob and that `NODE_PATH` has nothing at R — but if an in-place
//! write lands between that check and the backfill's batch write, the
//! backfill overwrites the writer's `NODE_PATH(R)` with the legacy blob's
//! stale path. Permanently: the blob is no longer legacy, so no re-run looks
//! at it again.
//!
//! So in-place writers hold the READ side of their branch's guard around
//! their batch write, and the backfill holds the WRITE side across its
//! re-check and its write. Either the in-place write lands first (the re-check
//! sees a rewritten blob and drops the entry) or after it (and overwrites the
//! backfill's entry, as any later write at R would). Ordinary writes, at a
//! fresh revision, never take it.
//!
//! Process-wide and striped by branch, like `lock_branch_record`: several
//! repository and transaction instances write one branch. A std lock, because
//! the backfill and the replicated upsert are synchronous — never hold a guard
//! across an `.await`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

const STRIPES: usize = 64;

static GUARDS: LazyLock<Vec<RwLock<()>>> =
    LazyLock::new(|| (0..STRIPES).map(|_| RwLock::new(())).collect());

fn stripe(tenant_id: &str, repo_id: &str, branch: &str) -> &'static RwLock<()> {
    let mut hasher = DefaultHasher::new();
    (tenant_id, repo_id, branch).hash(&mut hasher);
    &GUARDS[(hasher.finish() as usize) % STRIPES]
}

/// Held by an in-place record writer around its batch write.
pub(crate) fn in_place_write_guard(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> RwLockReadGuard<'static, ()> {
    stripe(tenant_id, repo_id, branch)
        .read()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Held by the `node_path` backfill across its re-check and its batch write.
pub(crate) fn backfill_write_guard(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> RwLockWriteGuard<'static, ()> {
    stripe(tenant_id, repo_id, branch)
        .write()
        .unwrap_or_else(PoisonError::into_inner)
}
