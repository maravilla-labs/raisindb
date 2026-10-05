//! The per-statement memo of fetched targets, and the statement's budget.
//!
//! A listing whose rows all reference the same header, footer and settings
//! nodes used to read those nodes once PER ROW. The memo lives as long as one
//! statement (the SQL engine keeps it on the `ExecutionContext`), so a target
//! shared by every row is read once.
//!
//! It is keyed by `(workspace, locator)` — the workspace is part of a target's
//! identity, so `/logo` in `assets` and `/logo` in `media` are two entries —
//! and by the read scope `(snapshot, locale, fields)`, because the same node
//! read in another language or trimmed to other fields is another value. A hit
//! is recorded under both its id and its path, so a reference by id and one by
//! path to the same node share one read.
//!
//! The entries are already filtered by row-level security for the statement's
//! caller. That is why a memo must never outlive the statement.

use super::budget::{self, ResolveBudget};
use super::walk::{self, RawRef, TargetRef, Totals};
use super::{doc, json_len};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// What a statement's reads are scoped to. Part of every memo key.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) struct ReadScope {
    pub(super) snapshot: HLC,
    /// `None` = the base language.
    pub(super) locale: Option<String>,
    pub(super) fields: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct MemoKey {
    read: Arc<ReadScope>,
    target: TargetRef,
}

impl MemoKey {
    pub(super) fn new(read: &Arc<ReadScope>, target: &TargetRef) -> Self {
        Self {
            read: read.clone(),
            target: target.clone(),
        }
    }
}

/// A fetched, translated, permitted and trimmed target, ready to inline.
#[derive(Debug)]
pub(super) struct Target {
    /// Exactly what is inlined (before its own references are): the node as
    /// the object `node_to_json_value_with_fields` renders, built from the
    /// stored values themselves (`doc.rs`).
    pub(super) value: PropertyValue,
    /// Its outgoing references, collected once from `value`.
    pub(super) refs: Vec<RawRef>,
    /// Serialized size, charged per inlined occurrence.
    pub(super) bytes: usize,
    id: String,
    path: String,
}

impl Target {
    pub(super) fn from_node(node: Node, fields: Option<&[String]>) -> Self {
        let id = node.id.clone();
        let path = node.path.clone();
        let mut value = node_value(node, fields);
        doc::make_walkable(&mut value);
        let refs = walk::distinct_refs(&value);
        let bytes = json_len::json_len(&value);
        Self {
            value,
            refs,
            bytes,
            id,
            path,
        }
    }
}

/// `node_to_json_value_with_fields`, as the value it renders: `id`, `name`,
/// `path`, `node_type`, then the properties (all, or only `fields`), a
/// property of the same name replacing an identity member. The node's own
/// property map becomes the object (no copy, no re-hash): the identity
/// members go in only where no property already has their name — which is
/// exactly "a property replaces it".
fn node_value(node: Node, fields: Option<&[String]>) -> PropertyValue {
    let Node {
        id,
        name,
        path,
        node_type,
        mut properties,
        ..
    } = node;
    if let Some(fields) = fields {
        properties.retain(|key, _| fields.iter().any(|f| f == key));
    }
    properties.reserve(4);
    for (key, value) in [
        ("id", id),
        ("name", name),
        ("path", path),
        ("node_type", node_type),
    ] {
        properties
            .entry(key.to_string())
            .or_insert(PropertyValue::String(value));
    }
    PropertyValue::Object(properties)
}

/// Counters for one statement's resolutions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResolveStats {
    /// Node reads issued to storage.
    pub reads: usize,
    /// Targets answered from the memo without a read.
    pub memo_hits: usize,
    /// Distinct targets charged against the budget.
    pub targets: usize,
    /// References replaced by a node, at every nesting level.
    pub occurrences: usize,
    /// Approximate bytes inlined.
    pub bytes: usize,
}

/// One statement's fetched targets and budget. See the module docs.
pub struct ResolveMemo {
    entries: Mutex<HashMap<MemoKey, Option<Arc<Target>>>>,
    budget: ResolveBudget,
    reads: AtomicUsize,
    hits: AtomicUsize,
    targets: AtomicUsize,
    occurrences: AtomicUsize,
    bytes: AtomicUsize,
}

impl Default for ResolveMemo {
    fn default() -> Self {
        Self::new(ResolveBudget::default())
    }
}

impl ResolveMemo {
    pub fn new(budget: ResolveBudget) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            budget,
            reads: AtomicUsize::new(0),
            hits: AtomicUsize::new(0),
            targets: AtomicUsize::new(0),
            occurrences: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
        }
    }

    pub fn stats(&self) -> ResolveStats {
        ResolveStats {
            reads: self.reads.load(Ordering::Relaxed),
            memo_hits: self.hits.load(Ordering::Relaxed),
            targets: self.targets.load(Ordering::Relaxed),
            occurrences: self.occurrences.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
        }
    }

    pub fn budget(&self) -> ResolveBudget {
        self.budget
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<MemoKey, Option<Arc<Target>>>> {
        // A panic elsewhere while holding the lock leaves a map that is still
        // consistent (every write is a single insert), so keep using it.
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `Some(entry)` when the target was already decided in this statement.
    pub(super) fn lookup(&self, key: &MemoKey) -> Option<Option<Arc<Target>>> {
        let hit = self.entries().get(key).cloned();
        if hit.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        }
        hit
    }

    /// Charge one distinct target against the budget, before it is read.
    pub(super) fn admit_target(&self) -> Result<()> {
        let n = self.targets.fetch_add(1, Ordering::Relaxed) + 1;
        if n > self.budget.max_targets {
            return Err(budget::targets_exceeded(self.budget.max_targets));
        }
        Ok(())
    }

    pub(super) fn count_read(&self) {
        self.reads.fetch_add(1, Ordering::Relaxed);
    }

    /// Record the decision for `target`, and for the other name of a found node
    /// (its path when read by id, its id when read by path).
    pub(super) fn store(
        &self,
        read: &Arc<ReadScope>,
        target: &TargetRef,
        entry: Option<Arc<Target>>,
    ) {
        let mut entries = self.entries();
        if let Some(found) = &entry {
            for alias in [&found.id, &found.path] {
                if alias.is_empty() || *alias == target.locator {
                    continue;
                }
                let alias = TargetRef {
                    workspace: target.workspace.clone(),
                    locator: alias.clone(),
                };
                entries
                    .entry(MemoKey::new(read, &alias))
                    .or_insert_with(|| entry.clone());
            }
        }
        entries.insert(MemoKey::new(read, target), entry);
    }

    /// What the statement may still inline.
    pub(super) fn allowance(&self) -> Totals {
        Totals {
            occurrences: self
                .budget
                .max_occurrences
                .saturating_sub(self.occurrences.load(Ordering::Relaxed)),
            bytes: self
                .budget
                .max_bytes
                .saturating_sub(self.bytes.load(Ordering::Relaxed)),
        }
    }

    /// Add one resolution's output to the statement's total.
    pub(super) fn charge(&self, totals: Totals) -> Result<()> {
        let occurrences = self
            .occurrences
            .fetch_add(totals.occurrences, Ordering::Relaxed)
            + totals.occurrences;
        let bytes = self.bytes.fetch_add(totals.bytes, Ordering::Relaxed) + totals.bytes;
        if occurrences > self.budget.max_occurrences {
            return Err(budget::occurrences_exceeded(self.budget.max_occurrences));
        }
        if bytes > self.budget.max_bytes {
            return Err(budget::bytes_exceeded(self.budget.max_bytes));
        }
        Ok(())
    }
}
