//! Where a background build request goes when the code asking has no storage
//! handle (a branch create, a lookup that found its index not built): the
//! storages whose job system runs, registered once it starts. Shared by the
//! automatic chains that take requests — the localized name index (plan
//! Phase 12) and the `property_index` rebuild (plan Phase 7b).

use super::RepairKind;
use crate::RocksDBStorage;
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
use std::time::{Duration, Instant};

/// `(tenant, repo, branch)`.
pub(crate) type BranchKey = (String, String, String);

/// Storages that can take a build request, by database path. Weak: a request
/// must never keep a closed storage alive.
fn requesters() -> &'static RwLock<HashMap<String, Weak<RocksDBStorage>>> {
    static MAP: OnceLock<RwLock<HashMap<String, Weak<RocksDBStorage>>>> = OnceLock::new();
    MAP.get_or_init(|| RwLock::new(HashMap::new()))
}

fn database_key(db: &DB) -> String {
    db.path().to_string_lossy().into_owned()
}

/// Register `storage` for build requests (its job system is running).
pub fn register_requester(storage: &Arc<RocksDBStorage>) {
    if let Ok(mut map) = requesters().write() {
        map.insert(database_key(storage.db()), Arc::downgrade(storage));
    }
}

/// Every registered storage still alive.
pub(crate) fn registered_storages() -> Vec<Arc<RocksDBStorage>> {
    requesters()
        .read()
        .map(|m| m.values().filter_map(Weak::upgrade).collect())
        .unwrap_or_default()
}

/// The registered storage over `db`, if its job system runs.
pub(crate) fn registered_storage_for(db: &DB) -> Option<Arc<RocksDBStorage>> {
    requesters()
        .read()
        .ok()?
        .get(&database_key(db))
        .and_then(Weak::upgrade)
}

/// Requests of one kind for one branch closer together than this collapse
/// into one: a site served by a fallback asks on every lookup, and each
/// request lists the job registry.
const REQUEST_DEBOUNCE: Duration = Duration::from_secs(5);

/// Whether a request of `kind` for `key` was made within
/// [`REQUEST_DEBOUNCE`] (and record this one).
pub(crate) fn debounced(kind: RepairKind, key: &BranchKey) -> bool {
    type Seen = HashMap<(&'static str, BranchKey), Instant>;
    static LAST: OnceLock<Mutex<Seen>> = OnceLock::new();
    let mut last = LAST
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    last.retain(|_, at| now.duration_since(*at) < REQUEST_DEBOUNCE);
    let entry = (kind.slug(), key.clone());
    if last.contains_key(&entry) {
        return true;
    }
    last.insert(entry, now);
    false
}
