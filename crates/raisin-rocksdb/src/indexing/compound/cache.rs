//! The storage behind `defs.rs`: per-branch entries with a per-branch
//! generation, so an answer read before an invalidation of THAT branch is
//! never stored after it — and a refresh or invalidation of one branch (or
//! one database: the cache is process-wide) never discards another's.
//!
//! That includes the bulk path. A checkpoint ingest copies NodeType records
//! into ONE database without events; the cache is registered with
//! `derived_cache_registry` as a DATABASE-scoped invalidator, so the ingest
//! drops that database's entries only. A process-wide drop would cold every
//! other database in the process, and on the apply path a cold definition
//! fails the workspace's compound indexes closed and forces a rebuild.

use super::defs::TypeIndexDefs;
use raisin_storage::BranchScope;
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// `{database path, tenant, repo, branch}`.
pub(super) type BranchKey = (String, String, String, String);

/// What a resolve saw when it started: the process-wide epoch (bumped by a
/// full invalidation), its database's epoch (bumped by an ingest into it)
/// and the branch's own generation.
pub(super) type Snapshot = (u64, u64, u64);

#[derive(Default)]
struct Inner {
    types: HashMap<BranchKey, HashMap<String, Arc<TypeIndexDefs>>>,
    generations: HashMap<BranchKey, u64>,
    databases: HashMap<String, u64>,
    epoch: u64,
}

impl Inner {
    fn snapshot(&self, key: &BranchKey) -> Snapshot {
        (
            self.epoch,
            self.databases.get(&key.0).copied().unwrap_or(0),
            self.generations.get(key).copied().unwrap_or(0),
        )
    }

    fn bump(&mut self, key: &BranchKey) {
        *self.generations.entry(key.clone()).or_insert(0) += 1;
    }
}

fn inner() -> &'static RwLock<Inner> {
    static INNER: OnceLock<RwLock<Inner>> = OnceLock::new();
    INNER.get_or_init(|| {
        raisin_core::register_database_invalidator(|database| match database {
            Some(path) => invalidate_database_path(path),
            None => invalidate_all(),
        });
        RwLock::new(Inner::default())
    })
}

fn database_key(db: &DB) -> String {
    db.path().to_string_lossy().into_owned()
}

pub(super) fn branch_key(db: &DB, scope: BranchScope<'_>) -> BranchKey {
    (
        database_key(db),
        scope.tenant_id.to_string(),
        scope.repo_id.to_string(),
        scope.branch.to_string(),
    )
}

pub(super) fn get(key: &BranchKey, node_type: &str) -> Option<Arc<TypeIndexDefs>> {
    inner().read().ok()?.types.get(key)?.get(node_type).cloned()
}

pub(super) fn snapshot(key: &BranchKey) -> Snapshot {
    inner()
        .read()
        .map(|i| i.snapshot(key))
        .unwrap_or((u64::MAX, u64::MAX, u64::MAX))
}

/// Store one answer, unless the branch was invalidated since `seen`.
pub(super) fn store(key: &BranchKey, node_type: &str, defs: Arc<TypeIndexDefs>, seen: Snapshot) {
    let Ok(mut i) = inner().write() else { return };
    if i.snapshot(key) != seen {
        return;
    }
    i.types
        .entry(key.clone())
        .or_default()
        .insert(node_type.to_string(), defs);
}

/// The type names cached for one branch.
pub(super) fn names(key: &BranchKey) -> Vec<String> {
    inner()
        .read()
        .ok()
        .and_then(|i| i.types.get(key).map(|m| m.keys().cloned().collect()))
        .unwrap_or_default()
}

/// Replace a branch's entries with `fresh` — unless it was invalidated since
/// `seen`, in which case it is only dropped (what was read may predate that
/// write). Either way the generation moves, so a resolve that read before
/// this point cannot store over the result. Returns whether it swapped.
pub(super) fn swap(
    key: &BranchKey,
    fresh: Vec<(String, Arc<TypeIndexDefs>)>,
    seen: Snapshot,
) -> bool {
    let Ok(mut i) = inner().write() else {
        return false;
    };
    let current = i.snapshot(key) == seen;
    i.bump(key);
    if current {
        i.types.insert(key.clone(), fresh.into_iter().collect());
    } else {
        i.types.remove(key);
    }
    current
}

/// Drop one branch's entries.
pub(super) fn invalidate(key: &BranchKey) {
    if let Ok(mut i) = inner().write() {
        i.bump(key);
        i.types.remove(key);
    }
}

/// Drop every entry of one database (a checkpoint ingest into it: NodeType
/// records arrived without events). A resolve that read before this point
/// cannot store after it.
pub fn invalidate_database(db: &DB) {
    invalidate_database_path(&database_key(db));
}

fn invalidate_database_path(path: &str) {
    if let Ok(mut i) = inner().write() {
        *i.databases.entry(path.to_string()).or_insert(0) += 1;
        i.types.retain(|key, _| key.0 != path);
    }
}

/// Drop everything, in every database of the process.
pub fn invalidate_all() {
    if let Ok(mut i) = inner().write() {
        i.epoch += 1;
        i.types.clear();
    }
}
