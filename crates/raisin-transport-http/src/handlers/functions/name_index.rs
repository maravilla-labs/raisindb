// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Name → path memo for function lookup.
//!
//! # Why this exists
//!
//! The HTTP function API addresses a function by NAME
//! (`POST /api/functions/{repo}/{name}/invoke`) while functions live at
//! arbitrary paths (`/lib/raisin/ai/weather`). Resolving one used to mean
//! `list_by_type("raisin:Function")` — loading EVERY function node in the
//! workspace and filtering in Rust. On an empty repo that is ~1.5 ms; on a
//! server with the builtin packages installed (~150 nodes) it is **170 ms, on
//! every single invocation**, dwarfing the function's own execution. Measured
//! with the phase timing in `invoke.rs`.
//!
//! # Why a validated memo rather than an invalidated cache
//!
//! This maps a name to a PATH, never to a node, and **every hit is re-read
//! through `get_by_path` and re-checked**. That has three consequences worth
//! stating, because they are what make it safe with no invalidation plumbing:
//!
//! 1. **RLS still applies.** The cached value is a path, not content. The
//!    point lookup runs with the caller's auth context exactly as the scan
//!    did, so a memo populated by one user cannot reveal a node to another.
//!    Caching the `Node` itself would have been a permission leak.
//! 2. **Stale entries self-heal.** Renamed, moved or deleted functions fail
//!    the re-check (or the lookup returns `None`) and fall through to the
//!    scan, which repopulates. The cost of staleness is one extra point
//!    lookup, never a wrong answer.
//! 3. **Misses are not cached.** A name that does not resolve today but is
//!    created a second later is found immediately; no negative caching means
//!    no "function exists but 404s until the TTL expires".
//!
//! The TTL therefore bounds memory, not correctness.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long an entry is kept. Correctness does not depend on this — see the
/// module docs — so it is long enough to cover any realistic burst.
const TTL: Duration = Duration::from_secs(300);

/// Hard cap on entries, so a workload that invokes many distinct names (or
/// probes nonexistent ones) cannot grow this without bound. On overflow the
/// map is cleared rather than evicted one-by-one: this is a memo, and
/// rebuilding it costs one scan per name actually in use.
const MAX_ENTRIES: usize = 4096;

type Key = (String, String, String, String);

struct Entry {
    path: String,
    stored: Instant,
}

static INDEX: LazyLock<Mutex<HashMap<Key, Entry>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn key(tenant: &str, repo: &str, branch: &str, name: &str) -> Key {
    (
        tenant.to_string(),
        repo.to_string(),
        branch.to_string(),
        name.to_string(),
    )
}

/// The path this name resolved to last time, if it is still fresh.
pub(super) fn get(tenant: &str, repo: &str, branch: &str, name: &str) -> Option<String> {
    let map = INDEX.lock().ok()?;
    let entry = map.get(&key(tenant, repo, branch, name))?;
    if entry.stored.elapsed() > TTL {
        return None;
    }
    Some(entry.path.clone())
}

/// Remember where a name resolved.
pub(super) fn put(tenant: &str, repo: &str, branch: &str, name: &str, path: &str) {
    let Ok(mut map) = INDEX.lock() else {
        return;
    };
    if map.len() >= MAX_ENTRIES {
        map.clear();
    }
    map.insert(
        key(tenant, repo, branch, name),
        Entry {
            path: path.to_string(),
            stored: Instant::now(),
        },
    );
}

/// Drop a name whose memo turned out to be wrong.
pub(super) fn forget(tenant: &str, repo: &str, branch: &str, name: &str) {
    if let Ok(mut map) = INDEX.lock() {
        map.remove(&key(tenant, repo, branch, name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_path_is_returned_and_can_be_forgotten() {
        put("t", "r", "main", "alpha", "/lib/alpha");
        assert_eq!(
            get("t", "r", "main", "alpha").as_deref(),
            Some("/lib/alpha")
        );
        forget("t", "r", "main", "alpha");
        assert_eq!(get("t", "r", "main", "alpha"), None);
    }

    #[test]
    fn entries_are_scoped_by_tenant_repo_and_branch() {
        put("t1", "r", "main", "shared", "/lib/one");
        put("t2", "r", "main", "shared", "/lib/two");
        assert_eq!(
            get("t1", "r", "main", "shared").as_deref(),
            Some("/lib/one")
        );
        assert_eq!(
            get("t2", "r", "main", "shared").as_deref(),
            Some("/lib/two")
        );
        // A different branch of the same repo is a different function space.
        assert_eq!(get("t1", "r", "dev", "shared"), None);
    }

    #[test]
    fn the_map_is_bounded() {
        for i in 0..(MAX_ENTRIES + 10) {
            put("bound", "r", "main", &format!("fn-{i}"), "/lib/x");
        }
        assert!(INDEX.lock().unwrap().len() <= MAX_ENTRIES);
    }
}
