//! The one writer for an ORDERED_CHILDREN entry whose label was decided
//! elsewhere.

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;

/// Put `child_id`'s ORDERED_CHILDREN entry under `parent_id` at `revision`,
/// and advance the parent's last-child metadata when `label` sorts after it.
///
/// The ONE writer for an entry whose label was decided elsewhere (a replicated
/// op's `cf_order_key`, a merge resolution's stored label): the metadata is
/// what `next_append_label` mints from, so an entry written without it makes
/// the next local append mint a label BELOW an existing child.
#[allow(clippy::too_many_arguments)]
pub(crate) fn put_ordered_child(
    batch: &mut rocksdb::WriteBatch,
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
    label: &str,
    revision: &HLC,
    child_id: &str,
    child_name: &str,
) -> Result<()> {
    let cf_ordered = cf_handle(db, cf::ORDERED_CHILDREN)?;
    let key = keys::ordered_child_key_versioned(
        tenant_id, repo_id, branch, workspace, parent_id, label, revision, child_id,
    );
    batch.put_cf(cf_ordered, key, child_name.as_bytes());

    let metadata_key =
        keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_id);
    if advances_last(
        db.get_cf(cf_ordered, &metadata_key)
            .ok()
            .flatten()
            .as_deref(),
        label,
    ) {
        batch.put_cf(cf_ordered, metadata_key, label.as_bytes());
    }
    Ok(())
}

/// Whether writing an entry under `label` advances a parent's cached LAST.
///
/// Only a PRESENT cache is advanced, and only past what it holds in editorial
/// order. An ABSENT cache stays absent: absence is a deliberate state (a merge
/// or a reorder-to-front invalidated it) that makes the next append run the
/// fallback scan for the true last label. Writing the label we were handed
/// there — typically an EXISTING child's middle label, re-put by a replicated
/// update or a merge resolution — made the next append mint inc(middle), a
/// fractional part that duplicates the next sibling's.
fn advances_last(cached: Option<&[u8]>, label: &str) -> bool {
    match cached {
        Some(existing) => super::sorts_after(label, &String::from_utf8_lossy(existing)),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::advances_last;

    #[test]
    fn replicated_middle_label_does_not_repopulate_an_invalidated_last_cache() {
        assert!(!advances_last(None, "a0::0000000000000001"));
    }

    #[test]
    fn last_cache_advances_only_past_what_it_holds() {
        assert!(advances_last(
            Some(b"a0::0000000000000009"),
            "a0V::0000000000000001"
        ));
        assert!(!advances_last(
            Some(b"a1::0000000000000001"),
            "a0V::0000000000000009"
        ));
    }
}
