//! The descending half of the ordered-children scan.
//!
//! Walking a label group backwards meets its revisions OLDEST first, so the
//! forward scan's "first entry seen is the newest" rule does not hold. The
//! descending scan buffers one label group at a time and resolves it here.

use super::super::helpers::is_tombstone;
use raisin_error::Result;
use std::collections::HashSet;

/// One buffered entry of a label group, for a descending scan.
pub(super) struct DescEntry {
    /// Raw descending-encoded revision: SMALLER bytes are NEWER.
    pub revision: Vec<u8>,
    pub child_id: String,
    pub label: String,
    pub value: Vec<u8>,
}

/// Emit one label group of a descending scan: the newest visible entry of
/// each child, live ones only, in exactly the reverse of the order a forward
/// scan emits them. Returns `Ok(false)` once `visit` asks to stop.
pub(super) fn flush_descending_group<F>(
    pending: &mut Vec<DescEntry>,
    seen_child_ids: &mut HashSet<String>,
    visit: &mut F,
) -> Result<bool>
where
    F: FnMut(&str, &str, &[u8]) -> Result<bool>,
{
    let mut newest: Vec<DescEntry> = Vec::new();
    for entry in pending.drain(..) {
        match newest.iter_mut().find(|e| e.child_id == entry.child_id) {
            Some(kept) if kept.revision <= entry.revision => {}
            Some(kept) => *kept = entry,
            None => newest.push(entry),
        }
    }
    // A forward scan emits a group in key order — (revision, child) — so the
    // reverse emits it in the reverse of that.
    newest.sort_by(|a, b| (&b.revision, &b.child_id).cmp(&(&a.revision, &a.child_id)));
    for e in newest {
        if is_tombstone(&e.value) || !seen_child_ids.insert(e.child_id.clone()) {
            continue;
        }
        if !visit(&e.child_id, &e.label, &e.value)? {
            return Ok(false);
        }
    }
    Ok(true)
}
