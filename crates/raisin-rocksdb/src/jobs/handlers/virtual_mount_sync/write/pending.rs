//! Process-local notes of nodes edited while their mount was busy syncing.
//!
//! A local edit enqueues a `VirtualMountWriteDrain`, which needs the per-mount
//! lease; if a sync run holds it the drain no-ops and the edit waits for "the
//! next run" — which on a mount importing hundreds of items back-to-back is
//! effectively never. The capture hook therefore NOTES the node here, and the
//! running sync — which already holds the lease — pushes noted nodes at its
//! page boundaries (`write::drain_pending`).
//!
//! Process-local by design, like the capture layer itself (see
//! `event_handler/vmount_capture.rs`): correctness is owned by the scheduled
//! drain and the reconcile walk, this only shortens latency. A note lost to a
//! restart, or stranded because the lease-holding sync runs on another cluster
//! node, falls back to exactly the pre-existing behaviour. A stale note (the
//! node was already drained) costs one point read and a converged skip.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

/// Cap per mount. A burst larger than this drops the overflow on the floor —
/// the scheduled drain nominates from the full index and covers it. Bounds the
/// map against a runaway writer; entries are removed when a mount's set drains
/// empty, so idle mounts do not accumulate (the `KeyedMutex` lesson).
const MAX_PENDING_PER_MOUNT: usize = 512;

fn map() -> &'static Mutex<HashMap<String, HashSet<String>>> {
    static PENDING: OnceLock<Mutex<HashMap<String, HashSet<String>>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// One key per mount, derived identically on the note side (capture hook) and
/// the take side (page-boundary drain). Mirrors the lease-key identity: config
/// branch, not target branch.
pub(crate) fn pending_key(tenant: &str, repo: &str, config_branch: &str, mount_id: &str) -> String {
    format!("{tenant}\0{repo}\0{config_branch}\0{mount_id}")
}

/// Note a node as edited under a mount. Idempotent; over the cap the note is
/// dropped (the scheduled drain covers it).
pub(crate) fn note(key: &str, node_id: &str) {
    let mut m = map().lock().unwrap_or_else(|p| p.into_inner());
    let set = m.entry(key.to_string()).or_default();
    if set.len() >= MAX_PENDING_PER_MOUNT {
        return;
    }
    set.insert(node_id.to_string());
}

/// Take up to `limit` noted nodes for this mount, leaving the rest for the
/// next boundary. Removes the mount's entry entirely once its set is empty.
pub(crate) fn take(key: &str, limit: usize) -> Vec<String> {
    let mut m = map().lock().unwrap_or_else(|p| p.into_inner());
    let Some(set) = m.get_mut(key) else {
        return Vec::new();
    };
    let ids: Vec<String> = set.iter().take(limit).cloned().collect();
    for id in &ids {
        set.remove(id);
    }
    if set.is_empty() {
        m.remove(key);
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_take_roundtrip_and_cleanup() {
        let key = pending_key("t", "r", "main", "m1");
        note(&key, "n1");
        note(&key, "n2");
        note(&key, "n1"); // idempotent
        let mut got = take(&key, 10);
        got.sort();
        assert_eq!(got, vec!["n1".to_string(), "n2".to_string()]);
        // Entry is removed once empty; a second take is a clean miss.
        assert!(take(&key, 10).is_empty());
    }

    #[test]
    fn take_respects_limit_and_leaves_rest() {
        let key = pending_key("t", "r", "main", "m2");
        for i in 0..5 {
            note(&key, &format!("n{i}"));
        }
        let first = take(&key, 2);
        assert_eq!(first.len(), 2);
        let rest = take(&key, 10);
        assert_eq!(rest.len(), 3);
    }
}
