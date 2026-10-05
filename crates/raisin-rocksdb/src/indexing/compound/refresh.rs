//! Re-resolving a branch's index definitions FROM STORAGE (never cache-first)
//! and swapping them in atomically — the one refresh, used after a NodeType
//! write (local repository, the applicator's schema arm, `Event::Schema`),
//! after a merge copied NodeType versions, by every compound build, by the
//! boot warm and by the cold-request drain.
//!
//! A cache-first "warm" never replaced a stale entry, so a build that started
//! while the cache lagged a declaration change derived its entries under the
//! old columns and stamped them `Ready` for the new one. Reading storage every
//! time closes that: the definitions a build writes with are the ones it
//! read, and the same answer goes into the cache.
//!
//! A refresh that finds a CACHED type's compound declarations changed marks
//! those index names `NotBuilt` in every workspace of the branch and requests
//! a local build: entries written under the old declaration (by this node's
//! writers, or replicated ones applied from the cache) cannot be trusted. On
//! the origin the NodeType write has marked them already
//! (`invalidate_changed_compound_state`); on a replica, after a merge, or for
//! a type first cached as unknown (no NodeType record yet), this is the only
//! place that notices.

use super::cache;
use super::defs::{changed_index_names, load, TypeIndexDefs};
use raisin_error::Result;
use raisin_storage::{BranchScope, NodeTypeRepository};
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;

/// The definitions of every type on the branch, plus `also` (types with no
/// NodeType record resolve to none and are cached as such), read from storage
/// and swapped into the cache. The map is what the caller builds with.
pub async fn fresh_branch<R: NodeTypeRepository>(
    db: &Arc<DB>,
    repo: &R,
    scope: BranchScope<'_>,
    also: &[&str],
) -> Result<HashMap<String, Arc<TypeIndexDefs>>> {
    let listed = repo.list(scope, None).await?;
    let mut names: Vec<&str> = listed.iter().map(|t| t.name.as_str()).collect();
    names.extend_from_slice(also);
    refresh(db, repo, scope, &names).await
}

/// Resolve every type on the branch into the cache (off the hot path: the
/// boot sweep, a compound build, the cold-request drain). Returns how many.
pub async fn warm_branch<R: NodeTypeRepository>(
    db: &Arc<DB>,
    repo: &R,
    scope: BranchScope<'_>,
) -> Result<usize> {
    Ok(fresh_branch(db, repo, scope, &[]).await?.len())
}

/// Re-resolve one branch's definitions after a schema change there — every
/// type that was cached plus `also` (the type just written) — and swap them
/// in ATOMICALLY: there is no moment where the branch reads cold, so a
/// replication apply arriving meanwhile is not failed closed for nothing. An
/// answer read before a concurrent invalidation is discarded (generation),
/// and one that fails to load leaves the branch dropped (cold), never stale.
pub async fn refresh_branch<R: NodeTypeRepository>(
    db: &Arc<DB>,
    repo: &R,
    scope: BranchScope<'_>,
    also: &[&str],
) -> Result<usize> {
    Ok(refresh(db, repo, scope, also).await?.len())
}

async fn refresh<R: NodeTypeRepository>(
    db: &Arc<DB>,
    repo: &R,
    scope: BranchScope<'_>,
    also: &[&str],
) -> Result<HashMap<String, Arc<TypeIndexDefs>>> {
    let key = cache::branch_key(db, scope);
    let seen = cache::snapshot(&key);
    let mut names = cache::names(&key);
    names.extend(also.iter().map(|n| n.to_string()));
    names.sort();
    names.dedup();
    let mut fresh = HashMap::with_capacity(names.len());
    for name in names {
        match load(repo, scope, &name).await {
            Ok(defs) => {
                fresh.insert(name, Arc::new(defs));
            }
            Err(e) => {
                cache::invalidate(&key);
                return Err(e);
            }
        }
    }
    let mut changed: Vec<String> = Vec::new();
    for (name, defs) in &fresh {
        if let Some(cached) = cache::get(&key, name) {
            changed.extend(changed_index_names(&cached.compound, &defs.compound));
        }
    }
    cache::swap(
        &key,
        fresh.iter().map(|(n, d)| (n.clone(), d.clone())).collect(),
        seen,
    );
    if !changed.is_empty() {
        changed.sort();
        changed.dedup();
        mark_changed(db, scope, &changed);
    }
    Ok(fresh)
}

/// Declarations changed under cached entries: fail the indexes closed and ask
/// for the local build that re-earns `Ready`.
fn mark_changed(db: &Arc<DB>, scope: BranchScope<'_>, names: &[String]) {
    let store = crate::compound_state::CompoundStateStore::new(db.clone());
    match store.mark_names_on_branch(scope.tenant_id, scope.repo_id, scope.branch, names) {
        Ok(workspaces) => {
            for workspace in workspaces {
                super::cold::request_build(
                    db,
                    scope.tenant_id,
                    scope.repo_id,
                    scope.branch,
                    &workspace,
                );
            }
            tracing::debug!(indexes = ?names, "compound declarations changed under the cache; marked NotBuilt");
        }
        Err(e) => {
            // Fail closed on the cache side too: the next write is cold.
            cache::invalidate(&cache::branch_key(db, scope));
            tracing::warn!(error = %e, "could not mark changed compound indexes NotBuilt");
        }
    }
}
