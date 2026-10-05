//! The cold-definition rule of the apply paths (plan Phase 8 step 3).
//!
//! A replicated upsert whose node types are not in the definitions cache
//! cannot write its compound entries (resolving them there is the NodeType
//! read the deadlock rule forbids). It must not skip silently either: it
//! marks the workspace's compound indexes `NotBuilt` in the same batch
//! (`CompoundStateStore::write_marking_stale`, the Phase 2.5 generation) and
//! REQUESTS a local build here. The job event handler drains the requests off
//! the hot path — it re-resolves the branch's definitions, the request's own
//! node types included, and sweeps the workspace's compound builds — so the
//! first cold write is also the last: every later one finds the cache warm
//! and maintains the index inline. The types matter: a node type with NO
//! NodeType record on this branch is listed by nothing, so a warm of the
//! listed types never caches it, and every write of such a node took the
//! cold path — a full workspace rebuild per write. Resolving the named types
//! caches them as "no definitions" (`defs::resolve`'s unknown type).

use rocksdb::DB;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// `{tenant, repo, branch, workspace}` that needs a compound build.
pub type ColdScope = (String, String, String, String);

type Pending = BTreeMap<(String, ColdScope), BTreeSet<String>>;

fn pending() -> &'static Mutex<Pending> {
    static PENDING: OnceLock<Mutex<Pending>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(BTreeMap::new()))
}

static COLD_WRITES: AtomicU64 = AtomicU64::new(0);

/// Compound build requests from writes that could not maintain the index
/// (cold definitions, legacy apply arms) since process start.
pub fn cold_definition_writes() -> u64 {
    COLD_WRITES.load(Ordering::Relaxed)
}

fn db_key(db: &DB) -> String {
    db.path().to_string_lossy().into_owned()
}

/// Record that `workspace` needs a local compound build (warm + sweep).
pub fn request_build(db: &DB, tenant_id: &str, repo_id: &str, branch: &str, workspace: &str) {
    request_build_for(db, tenant_id, repo_id, branch, workspace, &[]);
}

/// [`request_build`] from a write whose node `types` were cold: the drain
/// resolves them too.
pub fn request_build_for(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    types: &[&str],
) {
    COLD_WRITES.fetch_add(1, Ordering::Relaxed);
    let scope = (
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
        workspace.to_string(),
    );
    if let Ok(mut set) = pending().lock() {
        set.entry((db_key(db), scope))
            .or_default()
            .extend(types.iter().map(|t| t.to_string()));
    }
}

/// Whether a build of `workspace` is requested and not yet drained.
pub fn is_requested(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
) -> bool {
    let key = (
        db_key(db),
        (
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            workspace.to_string(),
        ),
    );
    pending().lock().is_ok_and(|set| set.contains_key(&key))
}

/// Take every request for `db`, with the node types each named.
pub fn drain(db: &DB) -> Vec<(ColdScope, Vec<String>)> {
    let me = db_key(db);
    let Ok(mut set) = pending().lock() else {
        return Vec::new();
    };
    let (mine, rest): (Pending, Pending) = std::mem::take(&mut *set)
        .into_iter()
        .partition(|((db, _), _)| *db == me);
    *set = rest;
    mine.into_iter()
        .map(|((_, scope), types)| (scope, types.into_iter().collect()))
        .collect()
}
