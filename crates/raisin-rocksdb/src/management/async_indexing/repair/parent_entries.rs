//! One parent's ORDERED_CHILDREN entries, read once and answered from memory.
//!
//! The NODES-tombstone pass asks, per delete, "which labels did this child
//! hold under this parent as of revision R". Answering that with a scan of the
//! parent's range per delete is quadratic on exactly the parents the repair
//! exists for — a parent whose many children were deleted (per-run job nodes).
//! Each parent is scanned once instead, kept in a bounded LRU, and the answers
//! come from memory; tombstones the pass writes are recorded into the cached
//! copy, so a later question sees them before they are committed.

use crate::repositories::nodes::parse_ordered_child_key;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{ReadOptions, DB};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;

/// One stored version of a `(label, child)` entry.
#[derive(Debug, Clone)]
struct Entry {
    label: String,
    revision: HLC,
    live: bool,
}

/// A parent's entries, grouped by child, each child's list in key order
/// (label ascending, revision newest first).
#[derive(Debug, Default)]
pub(super) struct ParentEntries {
    by_child: HashMap<String, Vec<Entry>>,
}

impl ParentEntries {
    fn load(db: &DB, prefix: &[u8]) -> Result<Self> {
        let cf = cf_handle(db, cf::ORDERED_CHILDREN)?;
        let mut opts = ReadOptions::default();
        opts.set_total_order_seek(true);
        opts.fill_cache(false);
        if let Some(upper) = crate::prefix_successor(prefix) {
            opts.set_iterate_upper_bound(upper);
        }
        let mut iter = db.raw_iterator_cf_opt(cf, opts);
        iter.seek(prefix);
        let mut out = Self::default();
        while iter.valid() {
            if let (Some(key), Some(value)) = (iter.key(), iter.value()) {
                if let Some(parsed) = parse_ordered_child_key(key, prefix) {
                    if let Some(revision) = parsed.revision() {
                        out.by_child
                            .entry(parsed.child_id.to_string())
                            .or_default()
                            .push(Entry {
                                label: parsed.order_label.to_string(),
                                revision,
                                live: !keys::is_tombstone_value(value),
                            });
                    }
                }
            }
            iter.next();
        }
        iter.status()
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        Ok(out)
    }

    /// The labels under which `child_id` is live, deciding each label by its
    /// newest entry at or below `at`.
    pub(super) fn live_labels(&self, child_id: &str, at: &HLC) -> Vec<String> {
        let Some(entries) = self.by_child.get(child_id) else {
            return Vec::new();
        };
        let mut decided: HashSet<&str> = HashSet::new();
        let mut live = Vec::new();
        for entry in entries {
            if &entry.revision > at || !decided.insert(entry.label.as_str()) {
                continue;
            }
            if entry.live {
                live.push(entry.label.clone());
            }
        }
        live
    }

    /// Record a tombstone this run wrote, so later questions see it.
    pub(super) fn record_tombstone(&mut self, child_id: &str, label: &str, revision: HLC) {
        let entries = self.by_child.entry(child_id.to_string()).or_default();
        entries.push(Entry {
            label: label.to_string(),
            revision,
            live: false,
        });
        entries.sort_by(|a, b| a.label.cmp(&b.label).then(b.revision.cmp(&a.revision)));
    }
}

/// A bounded LRU of [`ParentEntries`], keyed by the parent's prefix.
pub(super) struct ParentCache {
    lru: lru::LruCache<Vec<u8>, ParentEntries>,
}

impl ParentCache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            lru: lru::LruCache::new(NonZeroUsize::new(capacity.max(1)).unwrap()),
        }
    }

    /// The entries under `prefix`, loading them on first use.
    pub(super) fn entries(&mut self, db: &DB, prefix: &[u8]) -> Result<&mut ParentEntries> {
        if !self.lru.contains(prefix) {
            let loaded = ParentEntries::load(db, prefix)?;
            self.lru.put(prefix.to_vec(), loaded);
        }
        Ok(self
            .lru
            .get_mut(prefix)
            .expect("an entry put into the cache a line above is present"))
    }
}
