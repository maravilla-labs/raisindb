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
    let advance = match db.get_cf(cf_ordered, &metadata_key) {
        Ok(Some(existing)) => label > String::from_utf8_lossy(&existing).as_ref(),
        _ => true,
    };
    if advance {
        batch.put_cf(cf_ordered, metadata_key, label.as_bytes());
    }
    Ok(())
}
